use std::process::ExitCode;

use lapper_core::{Disposition, OffCause, Outcome, Run, RunResult, RunStatus, format_time};
use lapper_sim::{Faults, PRESETS, Scenario, Simulation, Tally, Verdict, simulate};

const USAGE: &str = "\
usage: lapper-sim [options]

options:
  --preset NAME   gymkhana | shared | dirt (default: gymkhana)
  --faults NAME   none | light | heavy (default: light)
  --seed N        random seed (default: 1)
  --sweep N       run seeds 1..=N and print totals
  --cars N        number of cars
  --heats N       runs per car
  --verbose       print every run
  --passages      print every passage and how it was assigned
";

struct Args {
    preset: String,
    faults: String,
    seed: u64,
    sweep: Option<u64>,
    cars: Option<u32>,
    heats: Option<u32>,
    verbose: bool,
    passages: bool,
}

fn parse_args() -> Result<Args, String> {
    let mut args = Args {
        preset: "gymkhana".into(),
        faults: "light".into(),
        seed: 1,
        sweep: None,
        cars: None,
        heats: None,
        verbose: false,
        passages: false,
    };
    let mut it = std::env::args().skip(1);
    while let Some(arg) = it.next() {
        let mut value = || it.next().ok_or(format!("{arg} needs a value"));
        match arg.as_str() {
            "--preset" => args.preset = value()?,
            "--faults" => args.faults = value()?,
            "--seed" => args.seed = number(&value()?)?,
            "--sweep" => args.sweep = Some(number(&value()?)?),
            "--cars" => args.cars = Some(number(&value()?)?),
            "--heats" => args.heats = Some(number(&value()?)?),
            "--verbose" | "-v" => args.verbose = true,
            "--passages" => args.passages = true,
            "--help" | "-h" => return Err(String::new()),
            _ => return Err(format!("unknown option: {arg}")),
        }
    }
    Ok(args)
}

fn number<T: std::str::FromStr>(s: &str) -> Result<T, String> {
    s.parse().map_err(|_| format!("not a number: {s}"))
}

fn main() -> ExitCode {
    let args = match parse_args() {
        Ok(args) => args,
        Err(message) => {
            if !message.is_empty() {
                eprintln!("{message}\n");
            }
            eprint!("{USAGE}");
            return ExitCode::from(2);
        }
    };
    let Some(faults) = Faults::by_name(&args.faults) else {
        eprintln!("unknown faults: {} (none | light | heavy)", args.faults);
        return ExitCode::from(2);
    };
    let Some(mut scenario) = Scenario::preset(&args.preset, faults) else {
        eprintln!("unknown preset: {} ({})", args.preset, PRESETS.join(" | "));
        return ExitCode::from(2);
    };
    if let Some(cars) = args.cars {
        scenario.cars = cars;
    }
    if let Some(heats) = args.heats {
        scenario.heats = heats;
    }

    println!(
        "preset: {}  faults: {}  cars: {}  heats: {}  max on course: {}",
        scenario.name, args.faults, scenario.cars, scenario.heats, scenario.max_on_course
    );
    match args.sweep {
        Some(n) => {
            let mut total = Tally::default();
            for seed in 1..=n {
                let (_, _, evaluation) = simulate(&scenario, seed);
                if args.verbose {
                    print_line(&format!("seed {seed:>4}"), &evaluation.tally);
                }
                total += evaluation.tally;
            }
            println!("seeds: 1..={n}");
            print_tally(&total);
        }
        None => {
            let (sim, outcome, evaluation) = simulate(&scenario, args.seed);
            println!("seed: {}", args.seed);
            if args.passages {
                print_passages(&sim, &outcome);
            }
            if args.verbose {
                print_runs(&sim, &outcome, &evaluation.judged);
            }
            print_tally(&evaluation.tally);
        }
    }
    ExitCode::SUCCESS
}

fn print_line(label: &str, t: &Tally) {
    println!(
        "{label}  runs {:>4}  correct {:>4}  dnf {:>3}  errors {:>3} (silent time {:>3}, car {:>3})  phantom {:>3}  flagged {:>3}",
        t.runs,
        t.correct,
        t.correct_dnf,
        t.errors(),
        t.silent_time,
        t.silent_car,
        t.phantom,
        t.flagged
    );
}

fn print_tally(t: &Tally) {
    let pct = |n: usize| {
        if t.runs == 0 {
            0.0
        } else {
            n as f64 * 100.0 / t.runs as f64
        }
    };
    println!(
        "runs:            {:>6}  (finished {}, off course {})",
        t.runs, t.finished, t.off
    );
    println!(
        "correct:         {:>6}  {:>6.2}%",
        t.correct,
        pct(t.correct)
    );
    println!(
        "correct DNF:     {:>6}  {:>6.2}%",
        t.correct_dnf,
        pct(t.correct_dnf)
    );
    println!(
        "wrong car:       {:>6}  {:>6.2}%",
        t.wrong_car,
        pct(t.wrong_car)
    );
    println!(
        "wrong time:      {:>6}  {:>6.2}%",
        t.wrong_time,
        pct(t.wrong_time)
    );
    println!(
        "finished, no time:{:>5}  {:>6.2}%",
        t.no_time,
        pct(t.no_time)
    );
    println!(
        "false finish:    {:>6}  {:>6.2}%",
        t.false_finish,
        pct(t.false_finish)
    );
    println!("lost:            {:>6}  {:>6.2}%", t.lost, pct(t.lost));
    println!(
        "silent, time:    {:>6}  {:>6.2}%  (time errors without a review mark)",
        t.silent_time,
        pct(t.silent_time)
    );
    println!(
        "silent, car:     {:>6}  {:>6.2}%  (car errors without a review mark)",
        t.silent_car,
        pct(t.silent_car)
    );
    println!("phantom runs:    {:>6}", t.phantom);
    println!("split errors:    {:>6}", t.split_errors);
    println!("flagged runs:    {:>6}", t.flagged);
}

fn print_runs(sim: &Simulation, outcome: &Outcome, judged: &[lapper_sim::Judged]) {
    println!(
        "{:>4} {:>4} {:>9} | {:>6} {:>4} {:>9} {:<14} | verdict",
        "#", "car", "truth", "run", "car", "result", "status"
    );
    for j in judged {
        let truth = &sim.truth[j.truth];
        let mut truth_result = match (truth.total(), truth.off_after) {
            (Some(t), _) => format_time(t),
            (None, Some(l)) => format!("off@{l}{}", if truth.reported { "r" } else { "" }),
            (None, None) => "?".into(),
        };
        // q: 列の外からのスタート、f: 運営が訂正した
        if truth.out_of_queue {
            truth_result.insert(0, 'q');
        }
        if truth.fixed {
            truth_result.insert(0, 'f');
        }
        let run = j.run.and_then(|id| outcome.run(id));
        let (id, car, result, status) = match run {
            Some(r) => (
                r.id.to_string(),
                r.car.map_or("-".into(), |c| c.0.to_string()),
                result_text(r),
                status_text(r),
            ),
            None => ("-".into(), "-".into(), "-".into(), "-".into()),
        };
        let mark = match (j.verdict, j.flagged) {
            (Verdict::Correct | Verdict::CorrectDnf, _) => "",
            (_, true) => " (flagged)",
            (_, false) => " (SILENT)",
        };
        println!(
            "{:>4} {:>4} {:>9} | {:>6} {:>4} {:>9} {:<14} | {:?}{}",
            j.truth, truth.car.0, truth_result, id, car, result, status, j.verdict, mark
        );
    }
    let matched: Vec<_> = judged.iter().filter_map(|j| j.run).collect();
    for r in outcome.runs.iter().filter(|r| !matched.contains(&r.id)) {
        println!(
            "{:>4} {:>4} {:>9} | {:>6} {:>4} {:>9} {:<14} | phantom",
            "-",
            "-",
            "-",
            r.id.to_string(),
            r.car.map_or("-".into(), |c| c.0.to_string()),
            result_text(r),
            status_text(r)
        );
    }
}

fn print_passages(sim: &Simulation, outcome: &Outcome) {
    println!(
        "{:>6} {:>4} {:>10} | {:<22} | truth",
        "id", "gate", "time", "assigned"
    );
    for p in &outcome.passages {
        let truth = sim.truth.iter().enumerate().find_map(|(i, t)| {
            let q = t.passages.iter().position(|&id| id == Some(p.id))?;
            Some(format!("#{i} car {} pos {q}", t.car.0))
        });
        let assigned = match p.disposition {
            Disposition::Assigned { run, position } => format!("{run} pos {position}"),
            Disposition::Debounced { by } => format!("debounced by {by}"),
            Disposition::Void => "void".into(),
            Disposition::Orphan => "orphan".into(),
        };
        println!(
            "{:>6} {:>4} {:>10} | {:<22} | {}",
            p.id.to_string(),
            p.gate.to_string(),
            format_time(p.time),
            assigned,
            truth.unwrap_or_else(|| "noise".into())
        );
    }
}

fn result_text(run: &Run) -> String {
    match run.result {
        RunResult::Time(t) => format_time(t),
        RunResult::Running => "running".into(),
        RunResult::MissCourse => "MC".into(),
        RunResult::Dnf => "DNF".into(),
    }
}

fn status_text(run: &Run) -> String {
    let base = match run.status {
        RunStatus::Running => "running",
        RunStatus::Finished => "finished",
        RunStatus::Off { cause, .. } => match cause {
            OffCause::Reported => "reported",
            OffCause::PushedOut { .. } => "pushed out",
            OffCause::TimedOut => "timed out",
        },
    };
    if run.flags.is_empty() {
        base.into()
    } else {
        format!("{base} !{}", run.flags.len())
    }
}
