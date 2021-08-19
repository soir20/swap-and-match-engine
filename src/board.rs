use std::collections::{HashMap, VecDeque, HashSet};
use crate::piece::{Piece, Direction, PieceType, ALL_DIRECTIONS};
use crate::position::Pos;
use crate::matching::{MatchPattern, Match};
use crate::bitboard::{BitBoard, BoardSize};
use enumset::EnumSet;

/// A group of positions on the board.
pub type PosSet = HashSet<Pos>;

/// Contains zero or many pieces and represents the current state
/// of the game.
///
/// Positions with larger y values are higher on the board. Positions
/// with larger x values are further right on the board.
///
/// There are three types of pieces: regular pieces, empty pieces,
/// and walls. Regular pieces may be movable in each of the four
/// cardinal directions: north, south, east, west. Empty pieces
/// represent a space with no piece, which is always movable. Walls
/// are always unmovable.
///
/// By default, the board is filled with walls. Users are responsible
/// for filling the board at the start of a game and after each match.
///
/// The board detects matches based on user-provided match patterns.
/// It does not have any match patterns by default. Patterns with
/// higher rank are preferred over those with lower rank.
///
/// The whole board is not scanned to check for matches. When a
/// piece is changed, either because it is set/overwritten or it
/// is swapped, it is marked as having changed. Then the changed
/// pieces are selectively checked for matches. Users should update
/// the board based on the positions provided in the match.
///
/// Swap rules define which pieces can be changed. By default, the
/// only swap rule in place is that pieces marked unmovable in a
/// direction cannot be moved any amount in that direction. **This
/// means that pieces further than one space away can be swapped by
/// default.**
///
/// The board's lack of default restrictions allows games to implement
/// their own unique or non-standard rules.
pub struct Board {
    size: BoardSize,
    patterns: Vec<MatchPattern>,
    swap_rules: Vec<Box<dyn Fn(&Board, Pos, Pos) -> bool>>,
    pieces: HashMap<PieceType, BitBoard>,
    empties: BitBoard,
    movable_directions: [BitBoard; 4],
    last_changed: VecDeque<Pos>
}

impl Board {

    /// Creates a new board.
    ///
    /// # Arguments
    ///
    /// * `size` - the size of the board. By default, all spaces are filled with walls,
    ///            so you do not need to use the whole board. Use the size closest to
    ///            the size you want.
    /// * `patterns` - the match patterns the board should use to detect matches. If
    ///                two patterns have the same rank, no order is guaranteed.
    /// * `swap_rules` - the swap rules that define whether two pieces can be swapped.
    ///                  If any rule returns false for two positions, the pieces are
    ///                  not swapped, and the swap method returns false. These rules
    ///                  are executed in the order provided after the default rule,
    ///                  so less expensive calculations should be done in earlier rules.
    pub fn new(size: BoardSize, mut patterns: Vec<MatchPattern>,
               mut swap_rules: Vec<Box<dyn Fn(&Board, Pos, Pos) -> bool>>) -> Board {
        patterns.sort_by(|a, b| b.rank().cmp(&a.rank()));
        swap_rules.insert(0, Box::from(Board::are_pieces_movable));

        Board {
            size,
            patterns,
            swap_rules,
            pieces: HashMap::new(),
            empties: BitBoard::new(size),
            movable_directions: [
                BitBoard::new(size),
                BitBoard::new(size),
                BitBoard::new(size),
                BitBoard::new(size)
            ],
            last_changed: VecDeque::new()
        }
    }

    /// Gets a piece at the given position on the board. If the position is
    /// outside the board, a wall is returned. By default, all pieces on the
    /// board are walls.
    ///
    /// # Arguments
    ///
    /// * `pos` - position of the piece to get
    pub fn piece(&self, pos: Pos) -> Piece {
        if self.empties.is_set(pos) {
            return Piece::Empty;
        }

        let possible_type = self.piece_type(pos);
        match possible_type {
            None => Piece::Wall,
            Some(piece_type) => Piece::Regular(piece_type, self.movable_directions(pos))
        }
    }

    /// Attempts to swap two pieces on the board. If any swap rule is broken (i.e. it
    /// results false), then the pieces will not be swapped, and this method will
    /// return false.
    ///
    /// If the swap is successful, both swapped positions will be marked for a match check.
    ///
    /// Swapping a piece in a direction in which it is marked unmovable is automatically
    /// a violation of the swap rules.
    ///
    /// Swapping with a piece that is empty is considered valid by default. The existing
    /// piece moves into the empty space while the other space is cleared. It is also valid
    /// to swap a piece with itself, though this has no effect on the board besides marking
    /// the piece for a match check.
    ///
    /// The order of two positions provided does not matter.
    ///
    /// # Arguments
    ///
    /// * `first` - the first position of a piece to swap
    /// * `second` - the second position of a piece to swap
    #[must_use]
    pub fn swap_pieces(&mut self, first: Pos, second: Pos) -> bool {
        if !self.swap_rules.iter().all(|rule| rule(self, first, second)) {
            return false;
        }

        self.last_changed.push_back(first);
        self.last_changed.push_back(second);

        self.empties = self.empties.swap(first, second);
        self.movable_directions = [
            self.movable_directions[0].swap(first, second),
            self.movable_directions[1].swap(first, second),
            self.movable_directions[2].swap(first, second),
            self.movable_directions[3].swap(first, second)
        ];

        let possible_first_type = self.piece_type(first);
        let possible_second_type = self.piece_type(second);

        // We don't want to undo the swap if both pieces are of the same type
        if possible_first_type != possible_second_type {
            if let Some(first_type) = possible_first_type {
                self.pieces.entry(first_type).and_modify(
                    |board| { *board = board.swap(first, second) }
                );
            }

            if let Some(second_type) = possible_second_type {
                self.pieces.entry(second_type).and_modify(
                    |board| { *board = board.swap(first, second) }
                );
            }
        }

        true
    }

    /// Replaces a piece at the given position and returns the previous piece.
    /// The space is marked as needing a match check. Swap rules do not apply
    /// and the replacement is always successful.
    ///
    /// # Arguments
    ///
    /// * `pos` - the position of the piece to replace
    /// * `piece` - the piece to put at the given position
    pub fn set_piece(&mut self, pos: Pos, piece: Piece) -> Piece {
        self.last_changed.push_back(pos);
        let old_piece = self.piece(pos);

        if let Some(piece_type) = self.piece_type(pos) {
            self.pieces.entry(piece_type).and_modify(
                |board| { *board = board.unset(pos) }
            );
        }

        match piece {
            Piece::Regular(piece_type, directions) => {
                self.pieces.entry(piece_type).and_modify(
                    |board| { *board = board.set(pos) }
                );
                self.empties = self.empties.unset(pos);
                self.set_movable_directions(pos, directions);
            },
            Piece::Empty => {
                self.empties = self.empties.set(pos);
                self.set_movable_directions(pos, ALL_DIRECTIONS);
            },
            Piece::Wall => {
                self.empties = self.empties.unset(pos);
                self.set_movable_directions(pos, EnumSet::new());
            }
        };

        old_piece
    }

    /// Gets the next match on the board. Matches from pieces that were changed
    /// earlier are returned first. Matches are always based on the current board
    /// state, not the board state when the match occurred.
    ///
    /// Pieces that were changed but did not create a match are skipped.
    ///
    /// Regardless of whether a match is found, each piece is unmarked for a
    /// match check, unless it has been marked multiple times.
    pub fn next_match(&mut self) -> Option<Match> {
        let mut next_pos;
        let mut next_match = None;

        while next_match.is_none() {
            next_pos = self.last_changed.pop_front()?;

            if let Some(piece_type) = self.piece_type(next_pos) {
                let board =  self.pieces.entry(piece_type).or_insert(BitBoard::new(self.size));

                next_match = self.patterns.iter().find_map(|pattern| {
                    let positions = Board::check_pattern(
                        board,
                        pattern.spaces(),
                        next_pos
                    )?;
                    Some(Match::new(pattern, next_pos, positions))
                });
            }
        }

        next_match
    }

    /// Gets the type of a piece at a certain position. If there is no regular piece
    /// at that position (i.e. it is empty or a wall), Option::None is returned.
    ///
    /// # Arguments
    ///
    /// * `pos` - the position of the piece whose type to find
    fn piece_type(&self, pos: Pos) -> Option<PieceType> {
        self.pieces.iter().find_map(|(&piece_type, board)|
            match board.is_set(pos) {
                true => Some(piece_type),
                false => None
            }
        )
    }

    /// Gets all of the movable directions for a piece at a given position.
    /// Empty pieces are always movable in all directions, while walls are
    /// movable in no directions.
    ///
    /// # Arguments
    ///
    /// * `pos` - the position of the piece whose movable directions to find
    fn movable_directions(&self, pos: Pos) -> EnumSet<Direction> {
        let mut directions = EnumSet::new();

        for direction in ALL_DIRECTIONS {
            if self.movable_directions[direction.value()].is_set(pos) {
                directions.insert(direction);
            }
        }

        directions
    }

    /// Sets the movable directions for a piece at a given position.
    ///
    /// # Arguments
    ///
    /// * `pos` - the position of the piece whose movable directions to set
    /// * `directions` the new movable directions of the piece
    fn set_movable_directions(&mut self, pos: Pos, directions: EnumSet<Direction>) {
        for direction in ALL_DIRECTIONS {
            if directions.contains(direction) {
                let ordinal = direction.value();
                self.movable_directions[ordinal] = self.movable_directions[ordinal].set(pos);
            }
        }
    }

    /// Checks if the pieces at two positions on the board are both movable in the
    /// direction in which they would be swapped.
    ///
    /// # Arguments
    ///
    /// * `first` - the position of the first piece to check
    /// * `second` - the position of the second piece to check
    fn are_pieces_movable(&self, first: Pos, second: Pos) -> bool {
        let is_first_movable = self.is_movable(first, second);
        let is_second_movable = self.is_movable(second, first);

        is_first_movable && is_second_movable
    }

    /// Checks if a piece is movable vertically and horizontally.
    ///
    /// # Arguments
    ///
    /// * `from` - the current position of the piece
    /// * `to` - the position where the piece will be moved
    fn is_movable(&self, from: Pos, to: Pos) -> bool {
        self.is_vertically_movable(from, to) && self.is_horizontally_movable(from, to)
    }

    /// Checks if a piece is vertically movable from one position to another.
    /// If there is no vertical change between the two positions, the piece
    /// is considered movable.
    ///
    /// # Arguments
    ///
    /// * `from` - the current position of the piece
    /// * `to` - the position where the piece will be moved
    fn is_vertically_movable(&self, from: Pos, to: Pos) -> bool {
        if to.y() > from.y() {
            return self.movable_directions[Direction::North.value()].is_set(from);
        } else if to.y() < from.y() {
            return self.movable_directions[Direction::South.value()].is_set(from);
        }

        true
    }

    /// Checks if a piece is horizontally movable from one position to another.
    /// If there is no horizontal change between the two positions, the piece
    /// is considered movable.
    ///
    /// # Arguments
    ///
    /// * `from` - the current position of the piece
    /// * `to` - the position where the piece will be moved
    fn is_horizontally_movable(&self, from: Pos, to: Pos) -> bool {
        if to.x() > from.x() {
            return self.movable_directions[Direction::East.value()].is_set(from);
        } else if to.x() < from.x() {
            return self.movable_directions[Direction::West.value()].is_set(from);
        }

        true
    }

    /// Checks for a pattern that includes a specific position on the board. Looks
    /// for all variants of a pattern (all possible patterns that include the required
    /// position). Returns the positions on the board that correspond to that pattern
    /// if there is a match.
    ///
    /// # Arguments
    ///
    /// * `board` - the board to check for a pattern
    /// * `pattern` - the set of relative positions that represent a pattern
    /// * `pos` - the position that must be included in a match
    fn check_pattern(board: &BitBoard, pattern: &PosSet, pos: Pos) -> Option<PosSet> {
        pattern.iter().find_map(|&original| Board::check_variant(board, pattern, pos - original))
    }

    /// Checks for a single variant of a pattern and returns the corresponding positions
    /// on the board if found.
    ///
    /// # Arguments
    ///
    /// * `board` - the board to check for a variant
    /// * `pattern` - the set of relative positions that represent a variant
    /// * `new_origin` - the origin to use for the pattern positions so that they
    ///                  correspond to actual positions on the board
    fn check_variant(board: &BitBoard, pattern: &PosSet, new_origin: Pos) -> Option<PosSet> {
        let grid_pos = Board::change_origin(pattern, new_origin);
        match grid_pos.iter().all(|&pos| board.is_set(pos)) {
            true => Some(grid_pos),
            false => None
        }
    }

    /// Changes the origin of a set of points.
    ///
    /// # Arguments
    ///
    /// * `positions` - the positions to change the origin of
    /// * `origin` - the new origin to use for the positions
    fn change_origin(positions: &PosSet, origin: Pos) -> PosSet {
        positions.iter().map(|&original| original + origin).collect()
    }

    /// Moves all the pieces in a column down to fill empty spaces directly beneath them.
    ///
    /// # Arguments
    ///
    /// * `x` - the x coordinate of the column to trickle
    fn trickle_column(&mut self, x: u8) {
        let movable_south = self.movable_directions[Direction::South.value()];
        let mut empty_spaces = VecDeque::new();

        for y in 0..self.size.height() {
            if self.empties.is_set(Pos::new(x, y)) {
                empty_spaces.push_back(y);
            } else if movable_south.is_set(Pos::new(x, y)) {
                if let Some(space_to_fill) = empty_spaces.pop_front() {
                    self.swap_pieces(Pos::new(x, y), Pos::new(x, space_to_fill));
                }
            } else {
                empty_spaces.clear();
            }
        }
    }

    /// Moves all pieces in the board diagonally and down until they can no longer be moved.
    /// Should be called after [trickle_column()](Board::trickle_column) is run on all columns.
    fn trickle_diagonally(&mut self) {
        for x in 0..self.size.width() {
            for y in 0..self.size.height() {
                let piece_pos = Pos::new(x, y);

                let mut previous_trickled_pos = piece_pos;
                let mut current_trickled_pos = self.trickle_piece(previous_trickled_pos);
                if previous_trickled_pos != current_trickled_pos {
                    self.trickle_column(x);
                }

                while previous_trickled_pos != current_trickled_pos {
                    previous_trickled_pos = current_trickled_pos;
                    current_trickled_pos = self.trickle_piece(previous_trickled_pos);
                }
            }
        }
    }

    /// Moves a piece diagonally, if possible, and then moves it down as far as possible.
    /// Returns the new position of the piece.
    ///
    /// # Arguments
    ///
    /// * `piece_pos` - the current position of the piece
    fn trickle_piece(&mut self, piece_pos: Pos) -> Pos {
        let mut diagonally_trickled_pos = self.trickle_piece_diagonally(piece_pos, true);
        if diagonally_trickled_pos == piece_pos {
            diagonally_trickled_pos = self.trickle_piece_diagonally(piece_pos, false);
        }

        self.trickle_piece_down(diagonally_trickled_pos)
    }

    /// Moves a piece one space down and one space horizontally if there is an
    /// empty space there. Returns the new position of the piece.
    ///
    /// # Arguments
    ///
    /// * `current_pos` - the current position of the piece to move
    /// * `to_west` - whether to move the piece west (or east if false)
    fn trickle_piece_diagonally(&mut self, current_pos: Pos, to_west: bool) -> Pos {
        let empty_pos = Board::move_pos_down_diagonally(current_pos, to_west);
        let is_empty_pos = self.is_within_board(empty_pos) && self.empties.is_set(empty_pos);

        let horizontal_dir_board = match to_west {
            true => self.movable_directions[Direction::West.value()],
            false => self.movable_directions[Direction::East.value()]
        };
        let vertical_dir_board = self.movable_directions[Direction::South.value()];
        let movable_board = horizontal_dir_board & vertical_dir_board;

        if !is_empty_pos || !(movable_board).is_set(current_pos) {
            return current_pos;
        }

        self.swap_pieces(current_pos, empty_pos);

        empty_pos
    }

    /// Moves a piece down until it is moved into the lowest empty space directly
    /// below it. Returns the new position of the piece
    ///
    /// # Arguments
    ///
    /// * `piece_pos` - the current position of the piece to move
    fn trickle_piece_down(&mut self, piece_pos: Pos) -> Pos {
        let vertical_dir_board = self.movable_directions[Direction::South.value()];
        if !vertical_dir_board.is_set(piece_pos){
            return piece_pos;
        }

        let mut next_y = piece_pos.y();
        while next_y > 0 && self.empties.is_set(Pos::new(piece_pos.x(), next_y - 1)) {
            next_y -= 1;
        }
        self.swap_pieces(piece_pos, Pos::new(piece_pos.x(), next_y));

        Pos::new(piece_pos.x(), next_y)
    }

    /// Moves a position one space down and one space horizontally.
    ///
    /// # Arguments
    ///
    /// * `pos` - the position to move
    /// * `to_west` - whether to move the position west (or east if false)
    fn move_pos_down_diagonally(pos: Pos, to_west: bool) -> Pos {
        match to_west {
            true => Pos::new(pos.x() - 1, pos.y() - 1),
            false => Pos::new(pos.x() + 1, pos.y() - 1)
        }
    }

    /// Checks if a given position is inside the board.
    ///
    /// # Arguments
    ///
    /// * `pos` - the position to check
    fn is_within_board(&self, pos: Pos) -> bool {
        pos.x() < self.size.width() && pos.y() < self.size.height()
    }

}
