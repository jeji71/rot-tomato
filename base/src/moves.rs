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

//! Definitions of moves, which can describe any legal playable move.

use crate::Board;

use super::{Piece, Square};

use std::{
    fmt::{Debug, Display, Formatter},
    mem::transmute,
};

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
/// The information of one move, containing its from- and to-squares, as well as
/// its promote type.
///
/// Internally, moves are represented as packed structures in a single unsigned
/// 16-bit integer. From MSB to LSB, the bits inside of a `Move` are as follows:
/// * 2 bits: flags (promotion, castling, or en passant)
/// * 2 bits: promote type
/// * 6 bits: from-square
/// * 6 bits: to-square
pub struct Move(u16);

impl Move {
    /// A sentinel value for a move which is illegal, or otherwise
    /// inexpressible. It is *strongly* recommended that `Option<Move>` is used
    /// instead of this.
    pub const BAD_MOVE: Move = Move(0xFFFF);

    /// The mask used to extract the flag bits from a move.
    const FLAG_MASK: u16 = 0xC000;

    /// The flag bits representing a move which is promotion.
    const PROMOTE_FLAG: u16 = 0x4000;

    /// The flag bits representing a move which is a castle.
    const CASTLE_FLAG: u16 = 0x8000;

    /// The flag bits representing a move which is en passant.
    const EN_PASSANT_FLAG: u16 = 0xC000;

    #[inline(always)]
    /// Make a new `Move` for a piece. Assumes that all the inputs are valid.
    ///
    /// # Panics
    ///
    /// This function will panic if the user requests that the move be tagged as
    /// both a castle and en passant move.
    pub const fn new(
        from_square: Square,
        to_square: Square,
        promote_type: Option<Piece>,
        castle: bool,
        en_passant: bool,
    ) -> Move {
        let mut bits = from_square as u16;
        bits |= (to_square as u16) << 6;

        if let Some(p) = promote_type {
            bits |= (p as u16) << 12 | Move::PROMOTE_FLAG; // promotion type
        }

        // apply flags
        bits |= match (castle, en_passant) {
            (false, false) => 0, // no special flag bits
            (false, true) => Move::EN_PASSANT_FLAG,
            (true, false) => Move::CASTLE_FLAG,
            (true, true) => panic!("move cannot be both castle and en passant"),
        };

        Move(bits)
    }

    #[inline(always)]
    /// Create a `Move` with no promotion type, which is not marked as having
    /// any extra special flags.
    pub const fn normal(from_square: Square, to_square: Square) -> Move {
        Move::new(from_square, to_square, None, false, false)
    }

    #[inline(always)]
    /// Create a `Move` with the given promotion type. The promote type must
    /// not be a pawn or a king.
    pub const fn promoting(from_square: Square, to_square: Square, promote_type: Piece) -> Move {
        Move::new(from_square, to_square, Some(promote_type), false, false)
    }

    #[inline(always)]
    /// Create a `Move` which is tagged as a castling move.
    pub const fn castling(from_square: Square, to_square: Square) -> Move {
        Move::new(from_square, to_square, None, true, false)
    }

    #[inline(always)]
    /// Create a `Move` which is tagged as a castling move.
    pub const fn en_passant(from_square: Square, to_square: Square) -> Move {
        Move::new(from_square, to_square, None, false, true)
    }

    #[inline(always)]
    /// Get the target square of this move.
    pub const fn to_square(&self) -> Square {
        // Masking out the bottom bits will make this always valid.
        unsafe { transmute(((self.0 >> 6) & 63u16) as u8) }
    }

    #[inline(always)]
    /// Get the square that a piece moves from to execute this move.
    pub const fn from_square(&self) -> Square {
        // Masking out the bottom bits will make this always valid
        unsafe { transmute((self.0 & 63u16) as u8) }
    }

    #[inline(always)]
    /// Determine whether this move is marked as a promotion.
    pub const fn is_promotion(&self) -> bool {
        self.0 & Move::FLAG_MASK == Move::PROMOTE_FLAG
    }

    #[inline(always)]
    /// Determine whether this move is marked as a castle.
    pub const fn is_castle(&self) -> bool {
        self.0 & Move::FLAG_MASK == Move::CASTLE_FLAG
    }

    #[inline(always)]
    /// Determine whether this move is marked as an en passant capture.
    pub const fn is_en_passant(&self) -> bool {
        self.0 & Move::FLAG_MASK == Move::EN_PASSANT_FLAG
    }

    #[inline(always)]
    /// Get the promotion type of this move. The resulting type will never be a
    /// pawn or a king.
    pub const fn promote_type(&self) -> Option<Piece> {
        if self.is_promotion() {
            Some(unsafe { std::mem::transmute(((self.0 >> 12) & 3u16) as u8) })
        } else {
            None
        }
    }

    /// Convert a move from its UCI representation. Requires the board the move
    /// was played on to determine extra flags about the move.
    pub fn from_uci(s: &str, board: &Board) -> Result<Move, &'static str> {
        if !(s.len() == 4 || s.len() == 5) {
            return Err("string was neither a normal move or a promotion");
        }
        let from_sq = Square::from_algebraic(&s[0..2])?;
        let to_sq = Square::from_algebraic(&s[2..4])?;
        let promote_type = if s.len() == 5 {
            // this is valid because we already checked the length of s
            let charcode = s.chars().nth(4).unwrap();
            let pt = Piece::from_code(charcode.to_ascii_uppercase());
            if pt == None {
                return Err("invalid promote type given");
            }
            pt
        } else {
            None
        };

        let is_castle = board[Piece::King].contains(from_sq) && from_sq.chebyshev_to(to_sq) > 1;

        let is_en_passant =
            board[Piece::Pawn].contains(from_sq) && board.en_passant_square == Some(to_sq);

        Ok(Move::new(
            from_sq,
            to_sq,
            promote_type,
            is_castle,
            is_en_passant,
        ))
    }

    /// Construct a UCI string version of this move.
    pub fn to_uci(&self) -> String {
        match self.promote_type() {
            None => format!("{}{}", self.from_square(), self.to_square()),
            Some(p) => format!(
                "{}{}{}",
                self.from_square(),
                self.to_square(),
                p.code().to_lowercase()
            ),
        }
    }

    #[inline(always)]
    /// Get a number representing this move uniquely. The value may change from
    /// version to version.
    pub const fn value(&self) -> u16 {
        self.0
    }

    #[inline(always)]
    /// Reconstruct a move based on its `value`. Should only be used with
    /// values returned from `Move::value()`.
    pub const fn from_val(val: u16) -> Move {
        Move(val)
    }
}

impl Debug for Move {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}{}", self.from_square(), self.to_square())?;
        if let Some(pt) = self.promote_type() {
            write!(f, "{}", pt.code())?;
        }
        if self.is_en_passant() {
            write!(f, " [e.p.]")?;
        }
        if self.is_castle() {
            write!(f, " [castle]")?;
        }
        Ok(())
    }
}

impl Display for Move {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self.promote_type() {
            None => write!(f, "{} -> {}", self.from_square(), self.to_square())?,
            Some(p) => write!(f, "{} -> {} ={}", self.from_square(), self.to_square(), p)?,
        };
        if self.is_en_passant() {
            write!(f, " [e.p.]")?;
        }
        if self.is_castle() {
            write!(f, " [castle]")?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_uci_move_normal() {
        let b = Board::default();
        let m = Move::from_uci("e2e4", &b).unwrap();
        println!("{m}");
        assert_eq!(m, Move::normal(Square::E2, Square::E4));
    }

    #[test]
    fn test_uci_move_promotion() {
        assert_eq!(
            Move::from_uci(
                "b7b8q",
                &Board::from_fen("r1b1kbnr/pPqppppp/2n5/8/8/8/P1PPPPPP/RNBQKBNR w KQkq - 1 5")
                    .unwrap(),
            )
            .unwrap(),
            Move::promoting(Square::B7, Square::B8, Piece::Queen),
        );
    }

    #[test]
    fn test_uci_move_capture() {
        assert_eq!(
            Move::from_uci(
                "c8c1",
                &Board::from_fen("1rr3k1/5pp1/3pp2p/p2n3P/1q1P4/1P1Q1N2/5PP1/R1R3K1 b - - 1 26")
                    .unwrap()
            )
            .unwrap(),
            Move::normal(Square::C8, Square::C1)
        );
    }

    #[test]
    fn test_uci_not_castle() {
        assert_eq!(
            Move::from_uci(
                "e1c1",
                &Board::from_fen("1rr3k1/5pp1/3pp2p/p2n3P/1q1P4/1P1Q1N2/5PP1/R3R1K1 w - - 0 26")
                    .unwrap()
            )
            .unwrap(),
            Move::normal(Square::E1, Square::C1)
        );
    }
}
