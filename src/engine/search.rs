use crate::base::algebraic::algebraic_from_move;
use crate::base::constants::{BLACK, WHITE};
use crate::base::util::opposite_color;
use crate::base::{Game, Move, MoveGenerator, PieceType, Square};
use crate::engine::positional::positional_evaluate;
use crate::engine::{Eval, EvaluationFn, MoveCandidacyFn};
use crate::engine::transposition::{EvalData, TTable};
use crate::Engine;

use std::cmp::{max, min};
use std::collections::HashMap;
use std::time::Instant;

/**
 * After going this many edges deep into the search tree, stop populating the
 * transposition table to save memory.
 */
const TRANSPOSITION_DEPTH_CUTOFF: i8 = 7;

/**
 * A stupid-simple engine which will evaluate the entire tree.
 */
pub struct Minimax {
    /**
     * The depth at which this algorithm will evaluate a position.
     */
    pub depth: i8,
    /**
     * The function used to evaluate the quality of a position.
     */
    pub evaluator: EvaluationFn,
    /**
     * The function used to determine which moves should be explored first.
     */
    pub candidator: MoveCandidacyFn,
    /**
     * The transposition table.
     */
    transpose_table: TTable,
    /**
     * The cumulative number of nodes evaluated in this evaluation event.
     */
    num_nodes_evaluated: u64,
    /**
     * The cumulative number of transpositions.
     */
    num_transpositions: u64,
}

impl Minimax {
    /**
     * Evaluate a position at a given depth. The depth is the number of plays
     * to make. Even depths are recommended for fair evaluations.
     */
    pub fn evaluate_at_depth(
        &mut self,
        depth: i8,
        alpha_in: Eval,
        beta_in: Eval,
        g: &mut Game,
        mgen: &MoveGenerator,
    ) -> Eval {
        self.num_nodes_evaluated += 1;

        let mut alpha = alpha_in;
        let mut beta = beta_in;

        if self.depth - depth < TRANSPOSITION_DEPTH_CUTOFF {
            if let Some(v) = self.transpose_table[g.get_board()] {
                if v.depth >= depth {
                    if v.lower_bound >= beta_in {
                        self.num_transpositions += 1;
                        return v.lower_bound;
                    }
                    if v.upper_bound <= alpha_in {
                        self.num_transpositions += 1;
                        return v.upper_bound;
                    }
                    alpha = max(alpha, v.lower_bound);
                    beta = min(beta, v.upper_bound);
                }
            }
        }

        if depth <= 0 || g.is_game_over(mgen) {
            let eval = self.quiesce(g, mgen, alpha_in, beta_in);
            self.transpose_table.store(
                *g.get_board(),
                EvalData {
                    depth: depth,
                    upper_bound: eval,
                    lower_bound: eval,
                },
            );
            return eval;
        }

        let player_to_move = g.get_board().player_to_move;

        let mut evaluation = match player_to_move {
            WHITE => Eval::MIN,
            BLACK => Eval::MAX,
            _ => Eval(0),
        };

        //these will be mutated by alpha-beta, but should not be put in the
        //transpoisition table
        let mut alpha_changing = alpha;
        let mut beta_changing = beta;

        let mut moves = mgen.get_moves(g.get_board());
        if depth > 1 {
            //negate because sort is ascending
            moves.sort_by_cached_key(|m| -(self.candidator)(g, mgen, *m));
        }

        for m in moves {
            g.make_move(m);
            let eval_for_m =
                self.evaluate_at_depth(depth - 1, alpha_changing, beta_changing, g, mgen);

            g.undo().ok();

            //alpha-beta pruning
            if player_to_move == WHITE {
                evaluation = max(evaluation, eval_for_m);
                if evaluation >= beta {
                    break;
                }
                alpha_changing = max(alpha_changing, evaluation);
            } else {
                //black moves on this turn
                evaluation = min(evaluation, eval_for_m);
                if evaluation <= alpha {
                    break;
                }
                beta_changing = min(beta_changing, evaluation);
            }
        }

        evaluation = evaluation.step_back();
        let mut lower_bound = Eval::MIN;
        let mut upper_bound = Eval::MAX;
        if evaluation <= alpha {
            upper_bound = evaluation;
        }
        if evaluation >= beta {
            lower_bound = evaluation;
        }
        if alpha < evaluation && evaluation < beta {
            lower_bound = evaluation;
            upper_bound = evaluation;
        }
        if self.depth - depth < TRANSPOSITION_DEPTH_CUTOFF {
            self.transpose_table.store(
                *g.get_board(),
                EvalData {
                    depth: depth,
                    lower_bound: lower_bound,
                    upper_bound: upper_bound,
                },
            );
        }
        return evaluation;
    }

    /**
     * Perform a quiescent (captures-only) search of the remaining moves.
     */
    fn quiesce(
        &mut self,
        g: &mut Game,
        mgen: &MoveGenerator,
        alpha_in: Eval,
        beta_in: Eval,
    ) -> Eval {
        self.num_nodes_evaluated += 1;

        let player = g.get_board().player_to_move;
        let enemy_occupancy = g.get_board().get_color_occupancy(opposite_color(player));
        let king_square = Square::from(g.get_board().get_type_and_color(PieceType::KING, player));
        let currently_in_check = mgen.is_square_attacked_by(
            g.get_board(), 
            king_square, opposite_color(player));
        let mut moves: Vec<Move> = g.get_moves(mgen);

        if !currently_in_check {
            moves = moves.into_iter()
            .filter(|m| enemy_occupancy.contains(m.to_square()))
            .collect();
        }

        if moves.len() == 0 {
            return (self.evaluator)(g, mgen);
        }

        moves.sort_by_cached_key(|m| -(self.candidator)(g, mgen, *m));

        let mut alpha = alpha_in;
        let mut beta = beta_in;

        let mut evaluation = match player {
            WHITE => Eval::MIN,
            _ => Eval::MAX,
        };

        for mov in moves {
            g.make_move(mov);
            let eval_for_mov = self.quiesce(g, mgen, alpha, beta);

            g.undo().ok();

            //alpha-beta pruning
            if player == WHITE {
                evaluation = max(evaluation, eval_for_mov);
                if evaluation >= beta {
                    break;
                }
                alpha = max(alpha, evaluation);
            } else {
                //black moves on this turn
                evaluation = min(evaluation, eval_for_mov);
                if evaluation <= alpha {
                    break;
                }
                beta = min(beta, evaluation);
            }
        }

        return evaluation;
    }

    /**
     * Clear out internal data.
     */
    pub fn clear(&mut self) {
        self.num_nodes_evaluated = 0;
    }
}

impl Default for Minimax {
    fn default() -> Minimax {
        let default_depth = 5;
        Minimax {
            depth: default_depth,
            evaluator: positional_evaluate,
            candidator: crate::engine::candidacy::candidacy,
            transpose_table: TTable::default(),
            num_nodes_evaluated: 0,
            num_transpositions: 0,
        }
    }
}

impl Engine for Minimax {
    #[inline]
    fn evaluate(&mut self, g: &mut Game, mgen: &MoveGenerator) -> Eval {
        self.num_nodes_evaluated = 0;
        self.num_transpositions = 0;
        let tic = Instant::now();
        let eval = self.evaluate_at_depth(self.depth, Eval::MIN, Eval::MAX, g, mgen);
        let toc = Instant::now();
        let nsecs = (toc - tic).as_secs_f64();
        println!(
            "evaluated {:.0} nodes in {:.0} secs ({:.0} nodes/sec) with {:0} transpositions",
            self.num_nodes_evaluated,
            nsecs,
            self.num_nodes_evaluated as f64 / nsecs,
            self.num_transpositions,
        );
        return eval;
    }

    fn get_evals(&mut self, g: &mut Game, mgen: &MoveGenerator) -> HashMap<Move, Eval> {
        let mut moves = g.get_moves(mgen);
        //negate because sort is ascending
        moves.sort_by_cached_key(|m| -(self.candidator)(g, mgen, *m));
        let mut evals = HashMap::new();
        for m in moves {
            g.make_move(m);
            let ev = self.evaluate(g, mgen);

            //this should never fail since we just made a move, but who knows?
            if let Ok(_) = g.undo() {
                evals.insert(m, ev);
            } else {
                println!("somehow, undoing failed on a game");
            }
            println!("{}: {}", algebraic_from_move(m, g.get_board(), mgen), ev);
        }
        return evals;
    }
}

#[cfg(test)]
pub mod tests {
    #[allow(unused_imports)]
    use super::*;
    #[allow(unused_imports)]
    use crate::base::fens::*;
    use crate::base::moves::Move;
    use std::collections::HashMap;

    #[test]
    /**
     * Test Minimax's evaluation of the start position of the game.
     */
    pub fn test_eval_start() {
        let mut g = Game::default();
        let mgen = MoveGenerator::new();
        let mut e = Minimax::default();

        println!("moves with evals are:");
        e.get_evals(&mut g, &mgen);
    }

    #[test]
    fn test_fried_liver() {
        let mut g = Game::from_fen(FRIED_LIVER_FEN).unwrap();
        let mgen = MoveGenerator::new();
        let mut e = Minimax::default();

        e.get_evals(&mut g, &mgen);
    }

    #[test]
    fn test_mate_in_1() {
        test_eval_helper(MATE_IN_1_FEN, Eval::mate_in(1));
    }

    #[test]
    fn test_mate_in_4_ply() {
        test_eval_helper(MATE_IN_4_FEN, Eval::mate_in(4));
    }

    #[test]
    fn test_my_special_puzzle() {
        let mut g = Game::from_fen(MY_PUZZLE_FEN).unwrap();
        let mgen = MoveGenerator::new();
        let mut e = Minimax::default();

        e.get_evals(&mut g, &mgen);
    }

    fn test_eval_helper(fen: &str, eval: Eval) {
        let mut g = Game::from_fen(fen).unwrap();
        let mgen = MoveGenerator::new();
        let mut e = Minimax::default();

        assert_eq!(e.evaluate(&mut g, &mgen), eval);
    }

    #[allow(dead_code)]
    /**
     * Print a map from moves to evals in a user-readable way.
     */
    fn print_move_map(map: &HashMap<Move, Eval>) {
        for (m, eval) in map {
            println!("{}:{}", m, eval);
        }
    }
}
