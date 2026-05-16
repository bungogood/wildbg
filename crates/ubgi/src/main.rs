use engine::composite::CompositeEvaluator;
use engine::dice::Dice;
use engine::evaluator::Evaluator;
use engine::multiply::MultiPlyEvaluator;
use engine::position::Position;
use logic::bg_move::BgMove;
use logic::wildbg_api::ScoreConfig;
use std::io::{self, BufRead, Write};

fn main() {
    let stdin = io::stdin();
    let mut stdout = io::stdout();

    let evaluator = match CompositeEvaluator::try_default() {
        Ok(evaluator) => evaluator,
        Err(err) => {
            reply(
                &mut stdout,
                &format!("error bad_state evaluator.init {err}"),
            );
            return;
        }
    };
    let evaluator_2ply = match CompositeEvaluator::try_default() {
        Ok(evaluator) => MultiPlyEvaluator { evaluator },
        Err(err) => {
            reply(
                &mut stdout,
                &format!("error bad_state evaluator.init {err}"),
            );
            return;
        }
    };

    let mut context = Context::default();

    for line in stdin.lock().lines() {
        let Ok(line) = line else {
            break;
        };
        let cmd = line.trim();
        if cmd.is_empty() {
            continue;
        }

        match cmd {
            "ubgi" => {
                reply(&mut stdout, "id name wildbg-ubgi 0.1");
                reply(&mut stdout, "id author wildbg contributors");
                reply(&mut stdout, "proto 0.2");
                reply(&mut stdout, "key game.variant enum backgammon backgammon");
                reply(&mut stdout, "key engine.ply int 1..2 1");
                reply(&mut stdout, "ubgiok");
            }
            "keys" => {
                reply(&mut stdout, "key game.variant enum backgammon backgammon");
                reply(&mut stdout, "key engine.ply int 1..2 1");
            }
            _ if cmd.starts_with("get ") => {
                let key = cmd.strip_prefix("get ").unwrap_or("").trim();
                match key {
                    "game.variant" => reply(&mut stdout, "value game.variant backgammon"),
                    "engine.ply" => reply(&mut stdout, "value engine.ply 1"),
                    _ => reply(&mut stdout, "error unsupported key"),
                }
            }
            _ if cmd.starts_with("set ") => {
                let rest = cmd.strip_prefix("set ").unwrap_or("");
                let mut it = rest.splitn(2, ' ');
                let key = it.next().unwrap_or("").trim();
                let value = it.next().unwrap_or("").trim();
                if key.is_empty() || value.is_empty() {
                    reply(&mut stdout, "error bad_command set");
                    continue;
                }
                match key {
                    "game.variant" => {
                        if value == "backgammon" {
                            context.position = None;
                            context.dice = None;
                        } else {
                            reply(&mut stdout, "error bad_value game.variant");
                        }
                    }
                    "engine.ply" => match value.parse::<usize>() {
                        Ok(1 | 2) => {}
                        _ => reply(&mut stdout, "error bad_value engine.ply"),
                    },
                    _ => reply(&mut stdout, "error unsupported key"),
                }
            }
            "isready" => reply(&mut stdout, "readyok"),
            "newgame" => {
                context.position = None;
                context.dice = None;
            }
            "setturn p0" | "setturn p1" | "stop" => {}
            "quit" => break,
            _ if cmd == "position xgid" || cmd.starts_with("position xgid ") => {
                reply(&mut stdout, "error unsupported position.xgid");
            }
            _ if cmd.starts_with("newsession ")
                || cmd.starts_with("setscore ")
                || cmd.starts_with("setcube ") => {}
            _ if cmd.starts_with("go") => {
                match handle_go(cmd, &context, &evaluator, &evaluator_2ply) {
                    Ok((info_line, bestmove_line)) => {
                        reply(&mut stdout, &info_line);
                        reply(&mut stdout, &bestmove_line);
                    }
                    Err(err) => reply(&mut stdout, &err),
                }
            }
            _ => {
                if let Some(id) = cmd.strip_prefix("position gnubgid ") {
                    match parse_position_id(id.trim()) {
                        Some(position) => context.position = Some(position),
                        None => reply(&mut stdout, "error bad_value position"),
                    }
                } else if let Some(rest) = cmd.strip_prefix("dice ") {
                    match parse_dice(rest) {
                        Some(dice) => context.dice = Some(dice),
                        None => reply(&mut stdout, "error bad_value dice"),
                    }
                } else {
                    reply(&mut stdout, "error bad_command unknown");
                }
            }
        }
    }
}

struct Context {
    position: Option<Position>,
    dice: Option<Dice>,
    score_config: ScoreConfig,
    ply: usize,
}

impl Default for Context {
    fn default() -> Self {
        Self {
            position: None,
            dice: None,
            score_config: ScoreConfig::MoneyGame,
            ply: 1,
        }
    }
}

fn parse_position_id(position_id: &str) -> Option<Position> {
    std::panic::catch_unwind(|| Position::from_id(position_id)).ok()
}

fn parse_dice(rest: &str) -> Option<Dice> {
    let parts: Vec<&str> = rest.split_whitespace().collect();
    if parts.len() != 2 {
        return None;
    }
    let d1 = parts[0].parse::<usize>().ok()?;
    let d2 = parts[1].parse::<usize>().ok()?;
    if !(1..=6).contains(&d1) || !(1..=6).contains(&d2) {
        return None;
    }
    Some(Dice::new(d1, d2))
}

fn validate_go(cmd: &str) -> Option<String> {
    if cmd == "go" || cmd == "go chequer" || cmd.starts_with("go chequer ") {
        return None;
    }
    if cmd.contains("role cube") {
        return Some("error unsupported role.cube".to_string());
    }
    if cmd.contains("role turn") {
        return Some("error unsupported role.turn".to_string());
    }
    if cmd.starts_with("go role ") {
        return Some("error bad_command role".to_string());
    }
    Some("error bad_command go".to_string())
}

fn handle_go(
    cmd: &str,
    context: &Context,
    evaluator: &CompositeEvaluator,
    evaluator_2ply: &MultiPlyEvaluator<CompositeEvaluator>,
) -> Result<(String, String), String> {
    if let Some(err) = validate_go(cmd) {
        return Err(err);
    }

    let position = context
        .position
        .ok_or_else(|| "error bad_state missing.position".to_string())?;
    let dice = context
        .dice
        .ok_or_else(|| "error bad_state missing.dice".to_string())?;

    let best_position = if context.ply <= 1 {
        evaluator.best_position(&position, &dice, context.score_config.value())
    } else {
        evaluator_2ply.best_position(&position, &dice, context.score_config.value())
    };
    let best_move = BgMove::new(&position, &best_position.sides_switched(), &dice);
    let best_move_text = format_move(best_move);

    Ok((
        format!("info role chequer pv {}", best_move_text),
        format!("bestmove {best_move_text}"),
    ))
}

fn format_move(bg_move: BgMove) -> String {
    let details = bg_move.into_details();
    if details.is_empty() {
        return "pass".to_string();
    }

    fn point_text(point: usize) -> &'static str {
        if point == 25 {
            "bar"
        } else if point == 0 {
            "off"
        } else {
            ""
        }
    }

    details
        .iter()
        .map(|d| {
            let from = d.from();
            let to = d.to();
            let from_text = point_text(from);
            let to_text = point_text(to);

            let from_part = if from_text.is_empty() {
                from.to_string()
            } else {
                from_text.to_string()
            };
            let to_part = if to_text.is_empty() {
                to.to_string()
            } else {
                to_text.to_string()
            };

            format!("{from_part}/{to_part}")
        })
        .collect::<Vec<String>>()
        .join(" ")
}

fn reply(out: &mut impl Write, line: &str) {
    let _ = writeln!(out, "{line}");
    let _ = out.flush();
}
