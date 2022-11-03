/*
  Fiddler, a UCI-compatible chess engine.
  Copyright (C) 2022 The Fiddler Authors (see AUTHORS.md file)

  Fiddler is free software: you can redistribute it and/or modify
  it under the terms of the GNU General Public License as published by
  the Free Software Foundation, either version 3 of the License, or
  (at your option) any later version.

  Fiddler is distributed in the hope that it will be useful,
  but WITHOUT ANY WARRANTY; without even the implied warranty of
  MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
  GNU General Public License for more details.

  You should have received a copy of the GNU General Public License
  along with this program.  If not, see <http://www.gnu.org/licenses/>.
*/

#![warn(clippy::pedantic)]
#![allow(clippy::inline_always)]

use std::{
    env,
    fs::File,
    io::{BufRead, BufReader},
    thread::scope,
    time::Instant,
};

use fiddler_base::{
    movegen::{KING_MOVES, KNIGHT_MOVES, PAWN_ATTACKS},
    Board, Color, Piece, Square, MAGIC,
};
use fiddler_engine::evaluate::{
    material,
    mobility::{ATTACKS_VALUE, MAX_MOBILITY},
    net_doubled_pawns, net_open_rooks, phase_of,
    pst::PST,
    DOUBLED_PAWN_VALUE, OPEN_ROOK_VALUE,
};
use libm::expf;

/// The input feature set of a board.
/// Each element is a (key, value) pair where the key is the index of the value
/// in the full feature vector.
type BoardFeatures = Vec<(usize, f32)>;

#[allow(clippy::similar_names)]
/// Run the main training function.
///
/// # Panics
///
/// This function will panic if the EPD training data is not specified or does
/// not exist.
pub fn main() {
    let args: Vec<String> = env::args().collect();
    // first argument is the name of the binary
    let path_str = &args[1..].join(" ");
    let mut weights = load_weights();
    // fuzz(&mut weights, 0.05);
    let learn_rate = 5.;

    let nthreads = 14;
    let tic = Instant::now();

    // construct the datasets.
    // Outer vector: each element for one datum
    // Inner vector: each element for one feature-quantity pair
    let input_sets = extract_epd(path_str).unwrap();

    let toc = Instant::now();
    println!("extracted data in {} secs", (toc - tic).as_secs());
    for i in 0..10000 {
        weights = train_step(&input_sets, &weights, learn_rate, nthreads).0;
        println!("iteration {i}...");
    }

    print_weights(&weights);
}

/// Expand an EPD file into a set of features that can be used for training.
fn extract_epd(location: &str) -> Result<Vec<(BoardFeatures, f32)>, Box<dyn std::error::Error>> {
    let file = File::open(location)?;
    let reader = BufReader::new(file);
    let mut data = Vec::new();

    for line_result in reader.lines() {
        let line = line_result?;
        let mut split_line = line.split('"');
        // first part of the split is the FEN, second is the score, last is just
        // a semicolon
        let fen = split_line.next().ok_or("no FEN given")?;
        let b = Board::from_fen(fen)?;
        let features = extract(&b);
        let score_str = split_line.next().ok_or("no result given")?;
        let score = match score_str {
            "1/2-1/2" => 0.5,
            "0-1" => 0.,
            "1-0" => 1.,
            _ => Err("unknown score string")?,
        };
        data.push((features, score));
    }

    Ok(data)
}

#[allow(clippy::similar_names, clippy::cast_precision_loss)]
/// Perform one step of PST training, and update the weights to reflect this.
/// Returns the weight vector MSE of the current epoch.
///
/// Inputs:
/// * `inputs`: a vector containing the input vector and the expected
///     evaluation.
/// * `weights`: the weight vector to train on.
/// * `learn_rate`: a coefficient on the speed at which the engine learns.
///
/// Each element of `inputs` must be the same length as `weights`.
fn train_step(
    inputs: &[(BoardFeatures, f32)],
    weights: &[f32],
    learn_rate: f32,
    nthreads: usize,
) -> (Vec<f32>, f32) {
    let tic = Instant::now();
    let chunk_size = inputs.len() / nthreads;
    let mut new_weights: Vec<f32> = weights.to_vec();
    let mut sum_se = 0.;
    scope(|s| {
        let mut grads = Vec::new();
        for thread_id in 0..nthreads {
            // start the parallel work
            let start = chunk_size * thread_id;
            grads.push(s.spawn(move || train_thread(&inputs[start..][..chunk_size], weights)));
        }
        for grad_handle in grads {
            let (sub_grad, se) = grad_handle.join().unwrap();
            sum_se += se;
            for i in 0..new_weights.len() {
                new_weights[i] -= learn_rate * sub_grad[i] / inputs.len() as f32;
            }
        }
    });
    let toc = Instant::now();
    let mse = sum_se / inputs.len() as f32;
    println!(
        "{} nodes in {:?}: {:.0} nodes/sec; mse {}",
        inputs.len(),
        (toc - tic),
        inputs.len() as f32 / (toc - tic).as_secs_f32(),
        mse
    );

    (new_weights, mse)
}

/// Construct the gradient vector for a subset of the input data.
/// Returns the sum of the squared error across this epoch.
fn train_thread(input: &[(BoardFeatures, f32)], weights: &[f32]) -> (Vec<f32>, f32) {
    let mut grad = vec![0.; weights.len()];
    let mut sum_se = 0.;
    for (features, sigm_expected) in input {
        let sigm_eval = sigmoid(features.iter().map(|&(idx, val)| val * weights[idx]).sum());
        let err = sigm_expected - sigm_eval;
        let coeff = -sigm_eval * (1. - sigm_eval) * err;
        // construct the gradient
        for &(idx, feat_val) in features {
            grad[idx] += feat_val * coeff;
        }
        sum_se += err * err;
    }

    (grad, sum_se)
}

#[inline(always)]
/// Compute the  sigmoid function of a variable.
/// `beta` is the horizontal scaling of the sigmoid.
///
/// The sigmoid function here is given by the LaTeX expression
/// `f(x) = \frac{1}{1 - \exp (- \beta x)}`.
fn sigmoid(x: f32) -> f32 {
    1. / (1. + expf(-x))
}

/// Load the weight value constants from the ones defined in the PST evaluation.
fn load_weights() -> Vec<f32> {
    let mut weights = Vec::new();
    for pt in Piece::NON_KING {
        let val = material::value(pt);
        weights.push(val.mg.float_val());
        weights.push(val.eg.float_val());
    }

    for pt in Piece::ALL {
        for rank in 0..8 {
            for file in 0..8 {
                let sq_idx = 8 * rank + file;
                let score = PST[pt as usize][sq_idx];
                weights.push(score.mg.float_val());
                weights.push(score.eg.float_val());
            }
        }
    }

    for pt in Piece::ALL {
        for count in 0..MAX_MOBILITY {
            let score = ATTACKS_VALUE[pt as usize][count as usize];
            weights.push(score.mg.float_val());
            weights.push(score.eg.float_val());
        }
    }

    weights.push(DOUBLED_PAWN_VALUE.mg.float_val());
    weights.push(DOUBLED_PAWN_VALUE.eg.float_val());

    weights.push(OPEN_ROOK_VALUE.mg.float_val());
    weights.push(OPEN_ROOK_VALUE.eg.float_val());

    weights
}

#[allow(clippy::cast_possible_truncation, clippy::similar_names)]
/// Print out a weights vector so it can be used as code.
fn print_weights(weights: &[f32]) {
    let paired_val = |name: &str, start: usize| {
        println!(
            "pub const {name}: Score = (Eval::centipawns({}), Eval::centipawns({}));",
            (weights[start] * 100.) as i16,
            (weights[start + 1] * 100.) as i16,
        );
    };

    // print material values
    paired_val("KNIGHT_VAL", 0);
    paired_val("BISHOP_VAL", 2);
    paired_val("ROOK_VAL", 4);
    paired_val("QUEEN_VAL", 6);
    paired_val("PAWN_VAL", 8);
    println!("-----");

    let mut offset = 10;

    // print PST
    println!("pub const PST: Pst = unsafe {{ transmute([");
    for pt in Piece::ALL {
        println!("    [ // {pt}");
        let pt_idx = offset + (128 * pt as usize);
        for rank in 0..8 {
            print!("        ");
            for file in 0..8 {
                let sq = Square::new(rank, file).unwrap();
                let mg_idx = pt_idx + (2 * sq as usize);
                let eg_idx = mg_idx + 1;
                let mg_val = (weights[mg_idx] * 100.) as i16;
                let eg_val = (weights[eg_idx] * 100.) as i16;
                print!("({mg_val}, {eg_val}), ");
            }
            println!();
        }
        println!("    ],");
    }
    println!("]) }};");
    println!("-----");

    // print mobility
    offset = 778;
    println!(
        "pub const ATTACKS_VALUE: [[Score; MAX_MOBILITY]; Piece::NUM] = unsafe {{ transmute(["
    );
    for pt in Piece::ALL {
        let pt_idx = offset + 2 * MAX_MOBILITY * pt as usize;
        println!("    [ // {pt}");
        for count in 0..MAX_MOBILITY {
            let count_idx = pt_idx + 2 * count;
            println!(
                "        ({}, {}), ",
                (weights[count_idx] * 100.) as i16,
                (weights[count_idx + 1] * 100.) as i16
            );
        }
        println!("    ],");
    }
    println!("]) }};");
    println!("-----");

    // print potpourri
    paired_val("DOUBLED_PAWN_VAL", 1114);
    paired_val("OPEN_ROOK_VAL", 1116);
}

#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    clippy::similar_names
)]
/// Extract a feature vector from a board.
/// The resulting vector will have dimension 1118.
/// The PST values can be up to 1 for a white piece on the given PST square, -1
/// for a black piece, or 0 for both or neither.
/// The PST values are then pre-blended by game phase.
///
/// The elements of the vector are listed by their indices as follows:
///
/// * 0..2: Knight quantity
/// * 2..4: Bishop quantity
/// * 4..6: Rook quantity
/// * 6..8: Queen quantity
/// * 8..10: Pawn quantity
/// * 10..138: Knight PST, paired (midgame, endgame) element-wise
/// * 138..266: Bishop PST
/// * 266..394: Rook PST
/// * 394..522: Queen PST
/// * 522..650: Pawn PST.
///     Note that the indices for the first and eight ranks do not matter.
/// * 650..778: King PST
/// * 778..1114: Mobility lookup.
///     This is not the most efficient representation, but it's easy to
///     implement.
///     The most major index is the piece type, then the number of attacked
///     squares, and lastly whether the evaluation is midgame or endgame.
/// * 1114..1116: Number of doubled pawns (mg, eg) weighted
///     (e.g. 1 if White has 2 doubled pawns and Black has 1)
/// * 1116..1118: Net number of open rooks
///
/// Ranges given above are lower-bound inclusive.
/// The representation is sparse, so each usize corresponds to an index in the
/// true vector. Zero entries will not be in the output.
fn extract(b: &Board) -> BoardFeatures {
    let mut features = Vec::with_capacity(28);
    let phase = phase_of(b);
    // Indices 0..8: non-king piece values
    for pt in Piece::NON_KING {
        let n_white = (b[pt] & b[Color::White]).len() as i8;
        let n_black = (b[pt] & b[Color::Black]).len() as i8;
        let net = n_white - n_black;
        if net != 0 {
            let idx = 2 * (pt as usize);
            features.push((idx, phase * f32::from(net)));
            features.push((idx, (1. - phase) * f32::from(net)));
        }
    }
    // just leave indices 9 and 10 unoccupied, I guess

    let mut offset = 10; // offset added to PST positions

    let bocc = b[Color::Black];
    let wocc = b[Color::White];

    // Get piece-square quantities
    for pt in Piece::ALL {
        let pt_idx = pt as usize;
        for sq in b[pt] {
            let alt_sq = sq.opposite();
            let increment = match (wocc.contains(sq), bocc.contains(alt_sq)) {
                (true, false) => 1.,
                (false, true) => -1.,
                _ => continue,
            };
            let idx = offset + 128 * pt_idx + 2 * (sq as usize);
            features.push((idx, phase * increment));
            features.push((idx + 1, (1. - phase) * increment));
        }
    }

    // New offset after everything in the PST.
    offset = 778;
    extract_mobility(b, &mut features, offset, phase);

    // Offset after mobility.
    offset = 1114;

    // Doubled pawns
    let doubled_count = net_doubled_pawns(b);
    if doubled_count != 0 {
        features.push((offset, f32::from(doubled_count) * phase));
        features.push((offset + 1, f32::from(doubled_count) * (1. - phase)));
    }

    // Open rooks
    let open_rook_count = net_open_rooks(b);
    if open_rook_count != 0 {
        features.push((offset + 2, f32::from(open_rook_count) * phase));
        features.push((offset + 3, f32::from(open_rook_count) * (1. - phase)));
    }

    features
}

/// Helper function to extract mobility information into the sparse feature
/// vector.
fn extract_mobility(b: &Board, features: &mut Vec<(usize, f32)>, offset: usize, phase: f32) {
    let white = b[Color::White];
    let black = b[Color::Black];
    let not_white = !white;
    let not_black = !black;
    let occupancy = white | black;
    let mut count = [[0i8; MAX_MOBILITY]; Piece::NUM];

    // count knight moves
    let knights = b[Piece::Knight];
    for sq in knights & white {
        let idx = usize::from((KNIGHT_MOVES[sq as usize] & not_white).len());
        count[Piece::Knight as usize][idx] += 1;
    }
    for sq in knights & black {
        let idx = usize::from((KNIGHT_MOVES[sq as usize] & not_black).len());
        count[Piece::Knight as usize][idx] -= 1;
    }

    // count bishop moves
    let bishops = b[Piece::Bishop];
    for sq in bishops & white {
        let idx = usize::from((MAGIC.bishop_attacks(occupancy, sq) & not_white).len());
        count[Piece::Bishop as usize][idx] += 1;
    }
    for sq in bishops & black {
        let idx = usize::from((MAGIC.bishop_attacks(occupancy, sq) & not_black).len());
        count[Piece::Bishop as usize][idx] -= 1;
    }

    // count rook moves
    let rooks = b[Piece::Rook];
    for sq in rooks & white {
        let idx = usize::from((MAGIC.rook_attacks(occupancy, sq) & not_white).len());
        count[Piece::Rook as usize][idx] += 1;
    }
    for sq in rooks & black {
        let idx = usize::from((MAGIC.rook_attacks(occupancy, sq) & not_black).len());
        count[Piece::Rook as usize][idx] -= 1;
    }

    // count queen moves
    let queens = b[Piece::Queen];
    for sq in queens & white {
        let idx = usize::from(
            ((MAGIC.rook_attacks(occupancy, sq) | MAGIC.bishop_attacks(occupancy, sq)) & not_white)
                .len(),
        );
        count[Piece::Queen as usize][idx] += 1;
    }
    for sq in rooks & black {
        let idx = usize::from(
            ((MAGIC.rook_attacks(occupancy, sq) | MAGIC.bishop_attacks(occupancy, sq)) & not_black)
                .len(),
        );
        count[Piece::Queen as usize][idx] -= 1;
    }

    // count net pawn moves
    // pawns can't capture by pushing, so we only examine their capture squares
    let pawns = b[Piece::Pawn];
    for sq in pawns & white {
        let idx = usize::from((PAWN_ATTACKS[Color::White as usize][sq as usize] & not_white).len());
        count[Piece::Pawn as usize][idx] += 1;
    }
    for sq in pawns & black {
        let idx = usize::from((PAWN_ATTACKS[Color::Black as usize][sq as usize] & not_black).len());
        count[Piece::Pawn as usize][idx] -= 1;
    }

    // king
    let white_king_idx =
        usize::from((KING_MOVES[b.king_sqs[Color::White as usize] as usize] & not_white).len());
    count[Piece::King as usize][white_king_idx] += 1;
    let black_king_idx =
        usize::from((KING_MOVES[b.king_sqs[Color::Black as usize] as usize] & not_black).len());
    count[Piece::King as usize][black_king_idx] -= 1;

    for pt in Piece::ALL {
        for idx in 0..MAX_MOBILITY {
            let num_mobile = count[pt as usize][idx];
            if num_mobile != 0 {
                let feature_idx = offset + 2 * (MAX_MOBILITY * pt as usize + idx);
                features.push((feature_idx, phase * f32::from(num_mobile)));
                features.push((feature_idx + 1, (1. - phase) * f32::from(num_mobile)));
            }
        }
    }
}
