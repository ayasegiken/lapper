use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::Arc;

use lapper_core::{Config, Micros, Mode, SECOND};
use lapper_protocol::inputs::{self, MAX_INTERMEDIATES};
use lapper_server::types::NewSession;
use lapper_server::{Hub, Store, api, feed, link};
use lapper_sim::{Faults, PRESETS, Scenario, generate};

const USAGE: &str = "\
usage: lapper-server [options]

options:
  --db PATH          SQLite database file (default: lapper.db)
  --listen ADDR      address to listen on (default: 0.0.0.0:3000)
  --web DIR          built web pages to serve (default: web/dist)
  --box ADDR         accept timing boxes over TCP (e.g. 0.0.0.0:4000);
                     their passages go to the newest session

new session (inputs: 0 start, 1-2 intermediates, 3 finish):
  --new-session NAME     create an empty session named NAME
  --intermediates N      intermediate timing points, 0-2 (default: 2;
                         with --simulate: the preset's)
  --same-gate            start and finish on the same gate (input 0)
  --strict               record passages in order without inference
                         (also with --simulate)
  --min-segment S[,S..]  minimum segment times in seconds: one for all
                         segments or one per segment (default: none)
  --max-on-course N      cars allowed on course at once (default: no limit)
  --max-run-time S       runs longer than S seconds time out (default: none)

simulation (creates a new session and feeds it in real time):
  --simulate NAME    busy | single | long
  --faults NAME      none | light | heavy (default: light)
  --seed N           random seed (default: 1)
  --speed X          playback speed (default: 1)
";

struct Args {
    db: PathBuf,
    listen: String,
    web: PathBuf,
    box_addr: Option<String>,
    new_session: Option<String>,
    intermediates: Option<usize>,
    same_gate: bool,
    strict: bool,
    min_segment: Option<Vec<Micros>>,
    max_on_course: Option<usize>,
    max_run_time: Option<Micros>,
    simulate: Option<String>,
    faults: String,
    seed: u64,
    speed: f64,
}

fn parse_args() -> Result<Args, String> {
    let mut args = Args {
        db: "lapper.db".into(),
        listen: "0.0.0.0:3000".into(),
        web: "web/dist".into(),
        box_addr: None,
        new_session: None,
        intermediates: None,
        same_gate: false,
        strict: false,
        min_segment: None,
        max_on_course: None,
        max_run_time: None,
        simulate: None,
        faults: "light".into(),
        seed: 1,
        speed: 1.0,
    };
    let mut it = std::env::args().skip(1);
    while let Some(arg) = it.next() {
        let mut value = || it.next().ok_or(format!("{arg} needs a value"));
        match arg.as_str() {
            "--db" => args.db = value()?.into(),
            "--listen" => args.listen = value()?,
            "--web" => args.web = value()?.into(),
            "--box" => args.box_addr = Some(value()?),
            "--new-session" => args.new_session = Some(value()?),
            "--intermediates" => {
                let value = value()?;
                let n = number(&value)?;
                if n > MAX_INTERMEDIATES {
                    return Err(format!(
                        "--intermediates must be 0-{MAX_INTERMEDIATES}: {value}"
                    ));
                }
                args.intermediates = Some(n);
            }
            "--same-gate" => args.same_gate = true,
            "--strict" => args.strict = true,
            "--min-segment" => {
                let value = value()?;
                let times = value.split(',').map(duration).collect::<Result<_, _>>()?;
                args.min_segment = Some(times);
            }
            "--max-on-course" => args.max_on_course = Some(number(&value()?)?),
            "--max-run-time" => args.max_run_time = Some(duration(&value()?)?),
            "--simulate" => args.simulate = Some(value()?),
            "--faults" => args.faults = value()?,
            "--seed" => args.seed = number(&value()?)?,
            "--speed" => args.speed = number(&value()?)?,
            "--help" | "-h" => return Err(String::new()),
            _ => return Err(format!("unknown option: {arg}")),
        }
    }
    if args.speed <= 0.0 {
        return Err("--speed must be positive".into());
    }
    if args.new_session.is_some() && args.simulate.is_some() {
        return Err("use either --new-session or --simulate".into());
    }
    let creates_session = args.new_session.is_some() || args.simulate.is_some();
    if args.strict && !creates_session {
        return Err("--strict needs --new-session or --simulate".into());
    }
    if args.intermediates.is_some() && !creates_session {
        return Err("--intermediates needs --new-session or --simulate".into());
    }
    let session_settings = args.same_gate
        || args.min_segment.is_some()
        || args.max_on_course.is_some()
        || args.max_run_time.is_some();
    if session_settings && args.new_session.is_none() {
        return Err(
            "--same-gate, --min-segment, --max-on-course and --max-run-time need --new-session"
                .into(),
        );
    }
    Ok(args)
}

fn number<T: std::str::FromStr>(s: &str) -> Result<T, String> {
    s.parse().map_err(|_| format!("not a number: {s}"))
}

/// 秒（小数も可）をマイクロ秒にする。
fn duration(s: &str) -> Result<Micros, String> {
    let seconds: f64 = number(s.trim())?;
    if !seconds.is_finite() || seconds < 0.0 {
        return Err(format!("not a duration in seconds: {s}"));
    }
    Ok((seconds * SECOND as f64).round() as Micros)
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
    match run(args).await {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("error: {message}");
            ExitCode::FAILURE
        }
    }
}

async fn run(args: Args) -> Result<(), String> {
    let store = Store::open(&args.db).map_err(|e| e.to_string())?;
    let hub = Arc::new(Hub::open(store).map_err(|e| e.to_string())?);

    if let Some(name) = &args.new_session {
        let session = hub
            .create_session(NewSession {
                name: name.clone(),
                config: new_session_config(&args)?,
                gate_names: None,
            })
            .map_err(|e| e.to_string())?;
        println!("created session {}: {}", session.id, session.name);
    }

    if let Some(addr) = &args.box_addr {
        let listener = tokio::net::TcpListener::bind(addr)
            .await
            .map_err(|e| format!("cannot listen on {addr}: {e}"))?;
        println!("accepting timing boxes on {addr}");
        tokio::spawn(link::serve(hub.clone(), listener));
    }

    if let Some(preset) = &args.simulate {
        let faults = Faults::by_name(&args.faults).ok_or(format!(
            "unknown faults: {} (none | light | heavy)",
            args.faults
        ))?;
        let scenario = scenario(&args, preset, faults)?;
        let sim = generate(&scenario, args.seed);
        let session = hub
            .create_session(NewSession {
                name: session_name(
                    &format!("simulation {preset} / {} / seed {}", args.faults, args.seed),
                    &args,
                ),
                config: config(sim.config.clone(), args.strict),
                gate_names: None,
            })
            .map_err(|e| e.to_string())?;
        println!(
            "simulating {preset} ({} events) into session {} at {}x",
            sim.log.len(),
            session.id,
            args.speed
        );
        let hub = hub.clone();
        let speed = args.speed;
        tokio::spawn(async move {
            match feed::feed(hub, session.id, sim, speed).await {
                Ok(()) => println!("simulation finished (session {})", session.id),
                Err(e) => eprintln!("simulation stopped: {e}"),
            }
        });
    }

    let web = args.web.is_dir().then_some(args.web.as_path());
    if web.is_none() {
        eprintln!(
            "note: {} not found; serving the API only (build the pages with `pnpm build` in web/)",
            args.web.display()
        );
    }
    let app = api::router(hub, web);
    let listener = tokio::net::TcpListener::bind(&args.listen)
        .await
        .map_err(|e| format!("cannot listen on {}: {e}", args.listen))?;
    println!("listening on http://{}", args.listen);
    axum::serve(listener, app).await.map_err(|e| e.to_string())
}

/// `--new-session` で作るセッションの設定。コースは標準の入力端子の割り当て（[`inputs`]）で作る。
fn new_session_config(args: &Args) -> Result<Config, String> {
    let intermediates = args.intermediates.unwrap_or(MAX_INTERMEDIATES);
    let mut config = Config::new(inputs::course(intermediates, args.same_gate));
    if let Some(times) = &args.min_segment {
        let segments = config.course.segments();
        config.min_segment = match times.len() {
            1 => vec![times[0]; segments],
            n if n == segments => times.clone(),
            n => {
                return Err(format!(
                    "--min-segment needs 1 or {segments} values for this course, got {n}"
                ));
            }
        };
    }
    config.max_on_course = args.max_on_course;
    config.max_run_time = args.max_run_time;
    Ok(self::config(config, args.strict))
}

/// シミュレーションの想定。`--intermediates` があれば中間の数を変える。
fn scenario(args: &Args, preset: &str, faults: Faults) -> Result<Scenario, String> {
    let scenario = Scenario::preset(preset, faults).ok_or(format!(
        "unknown preset: {preset} ({})",
        PRESETS.join(" | ")
    ))?;
    Ok(match args.intermediates {
        Some(n) => scenario.with_intermediates(n),
        None => scenario,
    })
}

/// セッションの名前。中間の数を変えたときや、そのまま記録するモードなら、それと分かるようにする。
fn session_name(name: &str, args: &Args) -> String {
    let mut name = name.to_string();
    match args.intermediates {
        Some(1) => name.push_str(" / 1 intermediate"),
        Some(n) => name.push_str(&format!(" / {n} intermediates")),
        None => {}
    }
    if args.strict {
        name.push_str(" (strict)");
    }
    name
}

/// セッションの設定。`strict` なら、推定をせずそのまま記録するモードにする。
fn config(mut config: Config, strict: bool) -> Config {
    if strict {
        config.mode = Mode::Strict;
    }
    config
}
