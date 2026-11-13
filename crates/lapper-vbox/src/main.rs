//! 仮想の計測ボックス。
//!
//! キーボードの数字キーを入力端子に見立て、押した瞬間に自分の時計で打刻して、本物の計測ボックスと同じやり取り
//! （[`lapper_protocol`]）でサーバーに送る。1 秒ごとの時計の通知、Ack が届くまでの再送、切断と再接続も本物と同じように動く。

use std::collections::BTreeMap;
use std::io::{self, Write};
use std::process::ExitCode;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use crossterm::event::{self, Event as TermEvent, KeyCode, KeyEventKind, KeyModifiers};
use crossterm::terminal;
use lapper_core::{Micros, SECOND};
use lapper_protocol::{BoxMessage, ServerMessage, decode, encode};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader, Lines};
use tokio::net::TcpStream;
use tokio::net::tcp::{OwnedReadHalf, OwnedWriteHalf};
use tokio::sync::mpsc;

const USAGE: &str = "\
usage: lapper-vbox [options]

options:
  --server ADDR   lapper-server's box address (default: 127.0.0.1:4000)
  --id NAME       box ID (default: vbox)
  --inputs N      number of inputs, 1-9 (default: 4)

keys:
  1-9             a passage on input 0-8
  d               disconnect / reconnect
  q, Esc, Ctrl-C  quit
";

/// 時計の通知の間隔。
const TICK: Duration = Duration::from_secs(1);

/// Ack が届かないとき、送り直すまでの時間。
const RESEND_AFTER: Duration = Duration::from_secs(2);

struct Args {
    server: String,
    id: String,
    inputs: u16,
}

fn parse_args() -> Result<Args, String> {
    let mut args = Args {
        server: "127.0.0.1:4000".into(),
        id: "vbox".into(),
        inputs: 4,
    };
    let mut it = std::env::args().skip(1);
    while let Some(arg) = it.next() {
        let mut value = || it.next().ok_or(format!("{arg} needs a value"));
        match arg.as_str() {
            "--server" => args.server = value()?,
            "--id" => args.id = value()?,
            "--inputs" => {
                let value = value()?;
                args.inputs = value
                    .parse()
                    .ok()
                    .filter(|n| (1..=9).contains(n))
                    .ok_or(format!("--inputs must be 1-9: {value}"))?;
            }
            "--help" | "-h" => return Err(String::new()),
            _ => return Err(format!("unknown option: {arg}")),
        }
    }
    Ok(args)
}

#[tokio::main]
async fn main() -> ExitCode {
    let args = match parse_args() {
        Ok(args) => args,
        Err(message) => {
            if !message.is_empty() {
                eprintln!("{message}\n");
            }
            eprint!("{USAGE}");
            return ExitCode::FAILURE;
        }
    };
    let raw = match RawMode::enable() {
        Ok(raw) => raw,
        Err(e) => {
            eprintln!("error: cannot read keys from this terminal: {e}");
            return ExitCode::FAILURE;
        }
    };
    let result = run(args).await;
    drop(raw);
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}

/// キーの操作。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Key {
    /// 入力端子 `input` の通過。`time` は押した瞬間のボックスの時刻。
    Input {
        input: u16,
        time: Micros,
    },
    ToggleOffline,
    Quit,
}

/// 押されたキーを操作にする。関係のないキーなら `None`。
fn key_for(code: KeyCode, modifiers: KeyModifiers, inputs: u16, time: Micros) -> Option<Key> {
    match code {
        KeyCode::Char('c') if modifiers.contains(KeyModifiers::CONTROL) => Some(Key::Quit),
        KeyCode::Char('q') | KeyCode::Esc => Some(Key::Quit),
        KeyCode::Char('d') => Some(Key::ToggleOffline),
        KeyCode::Char(c) => {
            let n = c.to_digit(10)? as u16;
            // then_some だと引数（n - 1）を先に計算してしまい、0 のとき桁あふれするので then にする
            (1..=inputs)
                .contains(&n)
                .then(|| Key::Input { input: n - 1, time })
        }
        _ => None,
    }
}

/// 送った通過のうち、まだ Ack が届いていないもの。
#[derive(Debug)]
struct Unacked {
    input: u16,
    time: Micros,
    /// 最後に送った時刻。まだ送っていなければ `None`。
    sent_at: Option<Instant>,
}

/// ボックスの状態。
struct VBox {
    id: String,
    boot: String,
    /// ボックスの時計の 0（起動した瞬間）。
    started: Instant,
    next_seq: u64,
    unacked: BTreeMap<u64, Unacked>,
}

impl VBox {
    fn new(id: String) -> VBox {
        VBox {
            id,
            boot: boot_id(),
            started: Instant::now(),
            next_seq: 0,
            unacked: BTreeMap::new(),
        }
    }

    /// ボックスの時計の今の時刻（起動してからのマイクロ秒）。
    fn now(&self) -> Micros {
        self.started.elapsed().as_micros() as Micros
    }

    /// 通過を記録し、その連番を返す。
    fn pass(&mut self, input: u16, time: Micros) -> u64 {
        let seq = self.next_seq;
        self.next_seq += 1;
        self.unacked.insert(
            seq,
            Unacked {
                input,
                time,
                sent_at: None,
            },
        );
        seq
    }

    /// 送っていない通過と、送ってから `resend_after` 以上たっても Ack がない通過を送る。
    async fn flush(&mut self, conn: &mut Connection, resend_after: Duration) -> io::Result<()> {
        for (&seq, unacked) in &mut self.unacked {
            if unacked
                .sent_at
                .is_some_and(|at| at.elapsed() < resend_after)
            {
                continue;
            }
            let message = BoxMessage::Passage {
                seq,
                input: unacked.input,
                time: unacked.time,
            };
            conn.send(&message).await?;
            unacked.sent_at = Some(Instant::now());
        }
        Ok(())
    }
}

/// 起動ごとに変わる ID。乱数のクレートを使わず、起動した時刻とプロセスの ID から作る。
fn boot_id() -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos());
    format!("{:x}-{:x}", nanos, std::process::id())
}

/// サーバーとの接続。
struct Connection {
    lines: Lines<BufReader<OwnedReadHalf>>,
    writer: OwnedWriteHalf,
}

impl Connection {
    /// 接続して Hello を送り、Ack のない通過をすべて送り直す。
    async fn open(addr: &str, vbox: &mut VBox) -> io::Result<Connection> {
        let stream = tokio::time::timeout(Duration::from_millis(500), TcpStream::connect(addr))
            .await
            .map_err(|_| io::Error::new(io::ErrorKind::TimedOut, "connection timed out"))??;
        let (reader, writer) = stream.into_split();
        let mut conn = Connection {
            lines: BufReader::new(reader).lines(),
            writer,
        };
        let hello = BoxMessage::Hello {
            box_id: vbox.id.clone(),
            boot: vbox.boot.clone(),
        };
        conn.send(&hello).await?;
        vbox.flush(&mut conn, Duration::ZERO).await?;
        Ok(conn)
    }

    async fn send(&mut self, message: &BoxMessage) -> io::Result<()> {
        self.writer.write_all(encode(message).as_bytes()).await
    }
}

/// 接続していれば次の行を読む。接続していなければ、いつまでも待つ（`select!` の中で使う）。
async fn read_line(conn: &mut Option<Connection>) -> io::Result<Option<String>> {
    match conn {
        Some(conn) => conn.lines.next_line().await,
        None => std::future::pending().await,
    }
}

async fn run(args: Args) -> io::Result<()> {
    let mut vbox = VBox::new(args.id.clone());
    let mut keys = spawn_key_reader(vbox.started, args.inputs);
    let mut conn: Option<Connection> = None;
    let mut offline = false;
    // 接続できないことを、毎秒ではなく一度だけ表示するため
    let mut reported_failure = false;
    let mut tick = tokio::time::interval(TICK);

    say(&format!(
        "lapper-vbox {} (boot {}) → {}。1〜{} で通過、d で切断 / 再接続、q で終了",
        vbox.id, vbox.boot, args.server, args.inputs
    ));
    loop {
        tokio::select! {
            key = keys.recv() => match key {
                Some(Key::Input { input, time }) => {
                    let seq = vbox.pass(input, time);
                    say(&format!("通過 {seq}: 入力 {input}  {}", format_time(time)));
                    if let Some(c) = &mut conn
                        && vbox.flush(c, RESEND_AFTER).await.is_err()
                    {
                        conn = None;
                        say("サーバーとの接続が切れました");
                    }
                }
                Some(Key::ToggleOffline) => {
                    offline = !offline;
                    if offline {
                        conn = None;
                        say("切断しました（通過は溜めておきます。d で再接続）");
                    } else {
                        say("再接続します");
                    }
                }
                Some(Key::Quit) | None => break,
            },
            line = read_line(&mut conn) => match line {
                Ok(Some(line)) => match decode::<ServerMessage>(&line) {
                    Ok(ServerMessage::Ack { seq }) => {
                        if vbox.unacked.remove(&seq).is_some() {
                            say(&format!("Ack {seq}（未確認 {} 件）", vbox.unacked.len()));
                        }
                    }
                    Err(e) => say(&format!("サーバーからの不明なメッセージ（{e}）: {line}")),
                },
                Ok(None) | Err(_) => {
                    conn = None;
                    say("サーバーとの接続が切れました");
                }
            },
            _ = tick.tick() => {
                if conn.is_none() && !offline {
                    match Connection::open(&args.server, &mut vbox).await {
                        Ok(c) => {
                            conn = Some(c);
                            reported_failure = false;
                            say(&format!("接続しました（未確認 {} 件を送りました）", vbox.unacked.len()));
                        }
                        Err(e) => {
                            if !reported_failure {
                                say(&format!("接続できません（{e}）。1 秒ごとに試します"));
                                reported_failure = true;
                            }
                        }
                    }
                }
                if let Some(c) = &mut conn {
                    let clock = BoxMessage::Clock { time: vbox.now() };
                    let sent = match c.send(&clock).await {
                        Ok(()) => vbox.flush(c, RESEND_AFTER).await,
                        Err(e) => Err(e),
                    };
                    if sent.is_err() {
                        conn = None;
                        say("サーバーとの接続が切れました");
                    }
                }
            }
        }
    }
    if !vbox.unacked.is_empty() {
        say(&format!(
            "Ack のない通過が {} 件残っています",
            vbox.unacked.len()
        ));
    }
    Ok(())
}

/// キーを読む専用のスレッドを起こす。crossterm のキーの読み取りは待ちっぱなしになるので、async の外で読む。
fn spawn_key_reader(started: Instant, inputs: u16) -> mpsc::UnboundedReceiver<Key> {
    let (tx, rx) = mpsc::unbounded_channel();
    std::thread::spawn(move || {
        loop {
            let Ok(event) = event::read() else { return };
            let TermEvent::Key(key) = event else { continue };
            if key.kind != KeyEventKind::Press {
                continue;
            }
            // 押した瞬間に打刻する
            let time = started.elapsed().as_micros() as Micros;
            let Some(key) = key_for(key.code, key.modifiers, inputs, time) else {
                continue;
            };
            if tx.send(key).is_err() || key == Key::Quit {
                return;
            }
        }
    });
    rx
}

/// 端末を 1 文字ずつ読むモード（raw モード）にする。値を捨てると元に戻る。
struct RawMode;

impl RawMode {
    fn enable() -> io::Result<RawMode> {
        terminal::enable_raw_mode()?;
        Ok(RawMode)
    }
}

impl Drop for RawMode {
    fn drop(&mut self) {
        let _ = terminal::disable_raw_mode();
    }
}

/// 1 行表示する（raw モードでは改行だけでは行頭に戻らないので `\r\n`）。
fn say(text: &str) {
    let mut out = io::stdout();
    let _ = write!(out, "{text}\r\n");
    let _ = out.flush();
}

/// ボックスの時刻を秒で表示する（例: `12.345678 秒`）。
fn format_time(time: Micros) -> String {
    format!("{}.{:06} 秒", time / SECOND, time % SECOND)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn digit_keys_are_inputs_within_range() {
        let none = KeyModifiers::NONE;
        assert_eq!(
            key_for(KeyCode::Char('1'), none, 4, 5),
            Some(Key::Input { input: 0, time: 5 })
        );
        assert_eq!(
            key_for(KeyCode::Char('4'), none, 4, 5),
            Some(Key::Input { input: 3, time: 5 })
        );
        assert_eq!(key_for(KeyCode::Char('5'), none, 4, 5), None);
        assert_eq!(key_for(KeyCode::Char('0'), none, 4, 5), None);
        assert_eq!(
            key_for(KeyCode::Char('c'), KeyModifiers::CONTROL, 4, 5),
            Some(Key::Quit)
        );
        assert_eq!(key_for(KeyCode::Char('c'), none, 4, 5), None);
        assert_eq!(
            key_for(KeyCode::Char('d'), none, 4, 5),
            Some(Key::ToggleOffline)
        );
    }

    #[test]
    fn passages_are_numbered_and_kept_until_acknowledged() {
        let mut vbox = VBox::new("test".into());
        assert_eq!(vbox.pass(0, 10), 0);
        assert_eq!(vbox.pass(3, 20), 1);
        assert_eq!(vbox.unacked.len(), 2);
        vbox.unacked.remove(&0);
        assert_eq!(vbox.unacked.keys().copied().collect::<Vec<_>>(), [1]);
    }

    #[test]
    fn time_is_shown_in_seconds() {
        assert_eq!(format_time(12_345_678), "12.345678 秒");
        assert_eq!(format_time(5), "0.000005 秒");
    }
}
