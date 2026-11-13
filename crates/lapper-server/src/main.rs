use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::Arc;

use lapper_core::{Config, Mode};
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
  --new-session NAME create an empty session with a preset course:
                     gymkhana | shared | dirt
  --strict           record passages in order without inference
                     (for --new-session and --simulate)

simulation (creates a new session and feeds it in real time):
  --simulate NAME    gymkhana | shared | dirt
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
    strict: bool,
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
        strict: false,
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
            "--strict" => args.strict = true,
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
    if args.strict && args.new_session.is_none() && args.simulate.is_none() {
        return Err("--strict needs --new-session or --simulate".into());
    }
    Ok(args)
}

fn number<T: std::str::FromStr>(s: &str) -> Result<T, String> {
    s.parse().map_err(|_| format!("not a number: {s}"))
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

    if let Some(preset) = &args.new_session {
        let scenario = Scenario::preset(preset, Faults::NONE).ok_or(format!(
            "unknown preset: {preset} ({})",
            PRESETS.join(" | ")
        ))?;
        let session = hub
            .create_session(NewSession {
                name: session_name(preset, args.strict),
                config: config(scenario.config(), args.strict),
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
        let scenario = Scenario::preset(preset, faults).ok_or(format!(
            "unknown preset: {preset} ({})",
            PRESETS.join(" | ")
        ))?;
        let sim = generate(&scenario, args.seed);
        let session = hub
            .create_session(NewSession {
                name: session_name(
                    &format!("simulation {preset} / {} / seed {}", args.faults, args.seed),
                    args.strict,
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

/// セッションの名前。そのまま記録するモードなら、それと分かるようにする。
fn session_name(name: &str, strict: bool) -> String {
    if strict {
        format!("{name} (strict)")
    } else {
        name.to_string()
    }
}

/// セッションの設定。`strict` なら、推定をせずそのまま記録するモードにする。
fn config(mut config: Config, strict: bool) -> Config {
    if strict {
        config.mode = Mode::Strict;
    }
    config
}
