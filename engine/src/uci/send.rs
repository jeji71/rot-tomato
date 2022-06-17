use super::{EngineInfo, OptionType, UciMessage};
use crate::Eval;

/// Construct a UCI message string from the engine to the GUI.
/// The message may be split into multiple lines (such as when handling
/// info-strings).
pub fn build_message(message: &UciMessage) -> String {
    match message {
        UciMessage::Id { name, author } => {
            let mut result = String::new();
            if let Some(n) = name {
                result += "id name ";
                result += n;
                result += "\n";
            }
            if let Some(a) = author {
                result += "id author ";
                result += a;
                result += "\n";
            }
            result
        }
        UciMessage::UciOk => "uciok\n".into(),
        UciMessage::ReadyOk => "readyok\n".into(),
        UciMessage::Option { name, opt } => build_option(name, opt),
        UciMessage::BestMove { m, ponder } => {
            let mut result = format!("bestmove {} ", m.to_uci());
            if let Some(pondermove) = ponder {
                result += &format!("ponder {pondermove}");
            }
            result += "\n";
            result
        }
        UciMessage::Info(info) => build_info(info),
    }
}

/// Helper function to build an output line to inform the GUI of an option.
fn build_option(name: &str, opt: &OptionType) -> String {
    let mut result = format!("option name {name} ");
    match opt {
        OptionType::Spin { default, min, max } => {
            result += &format!("type spin default {default} min {min} max {max}");
        }
        OptionType::String(s) => {
            result += "type string ";
            if let Some(st) = s {
                result += &format!("default {st} ");
            }
        }
        OptionType::Check(opt_default) => {
            result += "type check ";
            if let Some(default) = opt_default {
                result += &format!("default {default} ");
            }
        }
        OptionType::Combo { default, vars } => {
            result += "type combo ";
            if let Some(def_opt) = default {
                result += &format!("default {def_opt} ");
            }
            for var in vars.iter() {
                result += &format!("var {var} ");
            }
        }
        OptionType::Button => {
            result += "type button ";
        }
    }
    result += "\n";

    result
}

/// Build a set of messages for informing the GUI about facts of the engine.
fn build_info(infos: &[EngineInfo]) -> String {
    let mut result = String::from("info ");
    let mut new_line = false;
    for info in infos {
        if new_line {
            result += "\ninfo ";
            new_line = false;
        }
        match info {
            EngineInfo::Depth(depth) => result += &format!("depth {depth} "),
            EngineInfo::SelDepth(sd) => result += &format!("seldepth {sd} "),
            EngineInfo::Time(t) => result += &format!("time {} ", t.as_millis()),
            EngineInfo::Nodes(n) => result += &format!("nodes {n} "),
            EngineInfo::Pv(pv) => {
                result += "pv ";
                for m in pv.iter() {
                    result += &format!("{} ", m.to_uci());
                }
            }
            EngineInfo::MultiPv(id) => result += &format!("multipv {id} "),
            EngineInfo::Score {
                eval,
                is_lower_bound,
                is_upper_bound,
            } => {
                result += "score ";
                result += &match eval.moves_to_mate() {
                    Some(pl) => match eval > &Eval::DRAW {
                        true => format!("mate {pl} "),
                        false => format!("mate -{pl} "),
                    },
                    None => format!("cp {} ", eval.centipawn_val()),
                };
                if is_lower_bound & !is_upper_bound {
                    result += "lowerbound ";
                } else if *is_upper_bound {
                    result += "upperbound ";
                }
            }
            EngineInfo::CurrMove(m) => result += &format!("currmove {} ", m.to_uci()),
            EngineInfo::CurrMoveNumber(num) => result += &format!("currmovenumber {num} "),
            EngineInfo::HashFull(load) => result += &format!("hashfull {load} "),
            EngineInfo::NodeSpeed(speed) => result += &format!("nps {speed} "),
            // We split this info into two lines if
            EngineInfo::String(s) => {
                result += &format!("string {s}");
                new_line = true;
            }
        };
    }
    result + "\n"
}

#[cfg(test)]
mod tests {
    use super::*;

    use fiddler_base::{Eval, Move, Piece, Square};

    use std::time::Duration;

    #[test]
    /// Test an info message describing the current move.
    fn test_info_currmove() {
        assert_eq!(
            build_message(&UciMessage::Info(&[
                EngineInfo::CurrMove(Move::normal(Square::E2, Square::E4)),
                EngineInfo::CurrMoveNumber(1),
            ])),
            "info currmove e2e4 currmovenumber 1 \n"
        );
    }

    #[test]
    /// Test an info message describing a current move which is also a
    /// promotion.
    fn test_info_currmove_promotion() {
        assert_eq!(
            build_message(&UciMessage::Info(&[
                EngineInfo::CurrMove(Move::promoting(Square::E7, Square::E8, Piece::Queen)),
                EngineInfo::CurrMoveNumber(7),
            ])),
            "info currmove e7e8q currmovenumber 7 \n"
        );
    }

    #[test]
    /// Test an info message which is composed of many different pieces of
    /// information.
    fn test_info_composed() {
        assert_eq!(
            build_message(&UciMessage::Info(&[
                EngineInfo::Depth(2),
                EngineInfo::Score {
                    eval: Eval::pawns(2.14),
                    is_lower_bound: false,
                    is_upper_bound: false,
                },
                EngineInfo::Time(Duration::from_millis(1242)),
                EngineInfo::Nodes(2124),
                EngineInfo::NodeSpeed(34928),
                EngineInfo::Pv(&[
                    Move::normal(Square::E2, Square::E4),
                    Move::normal(Square::E7, Square::E5),
                    Move::normal(Square::G1, Square::F3),
                ]),
            ])),
            "info depth 2 score cp 214 time 1242 nodes 2124 nps 34928 pv e2e4 e7e5 g1f3 \n"
        )
    }

    #[test]
    /// Test an id message.
    fn test_id() {
        assert_eq!(
            build_message(&UciMessage::Id {
                name: Some("Fiddler"),
                author: Some("Clayton Ramsey"),
            }),
            "id name Fiddler\nid author Clayton Ramsey\n"
        )
    }

    #[test]
    /// Test an option message for a checkbox.
    fn test_option_check() {
        assert_eq!(
            build_message(&UciMessage::Option {
                name: "Nullmove",
                opt: OptionType::Check(Some(true)),
            }),
            "option name Nullmove type check default true \n"
        );
    }

    #[test]
    /// Test an option message for a spin-wheel.
    fn test_option_spin() {
        assert_eq!(
            build_message(&UciMessage::Option {
                name: "Selectivity",
                opt: OptionType::Spin {
                    default: 2,
                    min: 0,
                    max: 4
                },
            }),
            "option name Selectivity type spin default 2 min 0 max 4\n"
        )
    }

    #[test]
    /// Test an option message for a combo-box.
    fn test_option_combo() {
        assert_eq!(
            build_message(&UciMessage::Option {
                name: "Style",
                opt: OptionType::Combo {
                    default: Some("Normal"),
                    vars: &["Solid", "Normal", "Risky"],
                }
            }),
            "option name Style type combo default Normal var Solid var Normal var Risky \n"
        )
    }

    #[test]
    /// Test an option message for string input.
    fn test_option_string() {
        assert_eq!(
            build_message(&UciMessage::Option {
                name: "NalimovPath",
                opt: OptionType::String(Some("c:\\")),
            }),
            "option name NalimovPath type string default c:\\ \n"
        )
    }

    #[test]
    /// Test an option message for a button.
    fn test_option_button() {
        assert_eq!(
            build_message(&UciMessage::Option {
                name: "Clear Hash",
                opt: OptionType::Button,
            }),
            "option name Clear Hash type button \n"
        )
    }

    #[test]
    /// Test that best-moves are formatted correctly.
    fn test_bestmove() {
        assert_eq!(
            build_message(&UciMessage::BestMove {
                m: Move::normal(Square::E2, Square::E4),
                ponder: None
            }),
            "bestmove e2e4 \n"
        );
    }
}
