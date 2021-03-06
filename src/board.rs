use crate::bitboard::BitBoard;
use crate::matching::{MatchPattern, Match};
use crate::piece::{Piece, Direction, PieceType, ALL_DIRECTIONS};
use crate::position::Pos;

use std::collections::{VecDeque, HashSet, HashMap};
use std::fmt::{Debug, Formatter, Display};

use enumset::EnumSet;

/// Holds the current position of the pieces on the [Board] and the pieces
/// marked for a match check. BoardState is separate from the [Board] because
/// the [Board] is not (de)serializable. Thus, you can save the game by
/// saving the board state.
#[derive(Clone, Eq, PartialEq, Debug)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct BoardState {
    pub(crate) width: u8,
    pub(crate) height: u8,
    pub(crate) pieces: HashMap<PieceType, BitBoard>,
    pub(crate) empties: BitBoard,
    pub(crate) movable_directions: [BitBoard; 4],
    pub(crate) last_changed: VecDeque<Pos>
}

impl BoardState {

    /// Creates a default board state with a given size.
    ///
    /// All the pieces on the board are walls by default,
    /// and no pieces are marked for a match check.
    ///
    /// # Arguments
    ///
    /// * `width` - the horizontal size of the board to create
    /// * `height` - the vertical size of the board to create
    pub fn new(width: u8, height: u8) -> BoardState {
        BoardState {
            width,
            height,
            pieces: HashMap::new(),
            empties: BitBoard::new(width, height),
            movable_directions: [
                BitBoard::new(width, height),
                BitBoard::new(width, height),
                BitBoard::new(width, height),
                BitBoard::new(width, height)
            ],
            last_changed: VecDeque::new()
        }
    }

}

/// A group of positions on the board.
pub type PosSet = HashSet<Pos>;

/// A function that returns true if two pieces can be swapped.
pub type SwapRule = Box<dyn Fn(&Board, Pos, Pos) -> bool>;

/// Contains zero or many pieces and represents the current state
/// of the game.
///
/// Positions with larger y values are higher on the board. Positions
/// with larger x values are further right on the board. Positions start
/// at (0, 0), so a position at (16, 16) would be outside a 16x16 board
/// horizontally and vertically.
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
/// only swap rules in place is that pieces marked unmovable in a
/// direction cannot be moved any amount in that direction. **This
/// means that pieces further than one space away can be swapped
/// by default.**
///
/// The board's lack of default restrictions allows games to implement
/// their own unique or non-standard rules.
pub struct Board {
    patterns: Vec<MatchPattern>,
    swap_rules: Vec<SwapRule>,
    state: BoardState
}

impl Board {

    /// Creates a new board.
    ///
    /// # Arguments
    ///
    /// * `initial_state` - the initial state of the board. Create a state with a 
    ///                     size for brand new games. Otherwise, use a state 
    ///                     deserialized from your save format.
    /// * `patterns` - the match patterns the board should use to detect matches. If
    ///                two patterns have the same rank, no order is guaranteed.
    /// * `swap_rules` - the swap rules that define whether two pieces can be swapped.
    ///                  If any rule returns false for two positions, the pieces are
    ///                  not swapped, and the swap method returns false. These rules
    ///                  are executed in the order provided after the default rule,
    ///                  so less expensive calculations should be done in earlier rules.
    pub fn new(initial_state: BoardState, mut patterns: Vec<MatchPattern>,
               mut swap_rules: Vec<SwapRule>) -> Board {
        patterns.sort_by(|a, b| b.rank().cmp(&a.rank()));
        swap_rules.insert(0, Box::from(Board::are_pieces_movable));

        Board {
            patterns,
            swap_rules,
            state: initial_state
        }
    }

    /// Gets the current state of the board, which is (de)serializable and is
    /// useful for saving the board. Use other board methods to mutate the
    /// board's state.
    pub fn state(&self) -> &BoardState {
        &self.state
    }

    /// Gets a piece at the given position on the board. By default,
    /// all pieces on the board are walls.
    ///
    /// # Arguments
    ///
    /// * `pos` - position of the piece to get
    ///
    /// # Panics
    ///
    /// Panics if the provided position is outside the board.
    pub fn piece(&self, pos: Pos) -> Piece {
        if !self.is_within_board(pos) {
            panic!("Tried to get piece outside board: {}", pos);
        }

        if self.state.empties.is_set(pos) {
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
    /// to swap a piece with itself, though this has no effect on the board and does not
    /// mark the piece for a match check.
    ///
    /// The order of two positions provided does not matter.
    ///
    /// # Arguments
    ///
    /// * `first` - the first position of a piece to swap
    /// * `second` - the second position of a piece to swap
    ///
    /// # Panics
    ///
    /// Panics if either position is outside the board.
    #[must_use]
    pub fn swap_pieces(&mut self, first: Pos, second: Pos) -> bool {
        if !self.is_within_board(first) || !self.is_within_board(second) {
            panic!("Tried to swap piece outside board: {} with {}", first, second);
        }

        if !self.swap_rules.iter().all(|rule| rule(self, first, second)) {
            return false;
        }

        self.swap_always(first, second);
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
    ///
    /// # Panics
    ///
    /// Panics if the provided position is outside the board.
    pub fn set_piece(&mut self, pos: Pos, piece: Piece) -> Piece {
        if !self.is_within_board(pos) {
            panic!("Tried to set piece out of bounds: {}", pos);
        }

        self.state.last_changed.push_back(pos);
        let old_piece = self.piece(pos);

        if let Some(piece_type) = self.piece_type(pos) {
            self.state.pieces.entry(piece_type).and_modify(
                |board| board.unset(pos)
            );
        }

        match piece {
            Piece::Regular(piece_type, directions) => {
                let width = self.state.width;
                let height = self.state.height;
                self.state.pieces.entry(piece_type).and_modify(
                    |board| board.set(pos)
                ).or_insert_with(|| {
                    let mut board = BitBoard::new(width, height);
                    board.set(pos);
                    board
                });
                self.state.empties.unset(pos);
                self.set_movable_directions(pos, directions);
            },
            Piece::Empty => {
                self.state.empties.set(pos);
                self.set_movable_directions(pos, ALL_DIRECTIONS);
            },
            Piece::Wall => {
                self.state.empties.unset(pos);
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
            next_pos = self.state.last_changed.pop_front()?;

            let boards = &self.state.pieces;

            next_match = self.patterns.iter().find_map(|pattern| {
                if let Some(board) = boards.get(&pattern.piece_type()) {
                    let positions = Board::check_pattern(
                        board,
                        pattern.spaces(),
                        next_pos
                    )?;

                    return Some(Match::new(pattern, next_pos, positions));
                }

                None
            });
        }

        next_match
    }

    /// Moves all pieces down to fill the empty spaces below them.
    ///
    /// Pieces will move diagonally and down if there is an empty space there
    /// and if there is no piece in the column next to them that will fill
    /// that space. The piece will continue falling diagonally and then down
    /// until there are no more empty spaces to fill.
    ///
    /// A piece may fall diagonally left or right, but if both spaces are open,
    /// left is preferred.
    ///
    /// Pieces will not move past walls or other pieces that are unmovable and
    /// directly adjacent. However, pieces will move past walls that are diagonally
    /// adjacent.
    ///
    /// Does not fill empty spaces with new pieces.
    ///
    /// Marks all the spaces that change for a match check.
    ///
    /// Generates a sequence of moves in (from position, to position) format that
    /// makes the pieces fall naturally.
    pub fn trickle(&mut self) -> Vec<(Pos, Pos)> {
        let mut moves = Vec::new();

        for x in 0..self.state.width {
            moves.append(&mut self.trickle_column(x));
        }
        moves.append(&mut self.trickle_diagonally());

        moves
    }

    /// Replaces a space with a piece and moves it down to fill the empty
    /// spaces below it.
    ///
    /// If the piece provided is an empty piece or a wall, this method performs
    /// identically to [set_piece()].
    ///
    /// A regular piece will move diagonally and down if there is an empty space
    /// there. The piece will continue falling diagonally and then down until
    /// there are no more empty spaces to fill.
    ///
    /// The piece may fall diagonally left or right, but if both spaces are open,
    /// left is preferred.
    ///
    /// The piece will not move past walls or other pieces that are unmovable and
    /// directly adjacent. However, it will move past walls that are diagonally
    /// adjacent.
    ///
    /// Does not fill empty spaces with new pieces.
    ///
    /// Marks all the spaces that change for a match check.
    ///
    /// Generates a sequence of moves in (from position, to position) format that
    /// makes the piece fall naturally.
    pub fn add_and_trickle(&mut self, pos: Pos, piece: Piece) -> Vec<(Pos, Pos)> {
        self.set_piece(pos, piece);
        self.trickle_piece(pos, false)
    }

    /// Gets the type of a piece at a certain position. If there is no regular piece
    /// at that position (i.e. it is empty or a wall), Option::None is returned.
    ///
    /// # Arguments
    ///
    /// * `pos` - the position of the piece whose type to find
    fn piece_type(&self, pos: Pos) -> Option<PieceType> {
        self.state.pieces.iter().find_map(|(&piece_type, board)|
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
            if self.state.movable_directions[direction as usize].is_set(pos) {
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
            let ordinal = direction as usize;
            if directions.contains(direction) {
                self.state.movable_directions[ordinal].set(pos);
            } else {
                self.state.movable_directions[ordinal].unset(pos);
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
            return self.state.movable_directions[Direction::North as usize].is_set(from);
        } else if to.y() < from.y() {
            return self.state.movable_directions[Direction::South as usize].is_set(from);
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
            return self.state.movable_directions[Direction::East as usize].is_set(from);
        } else if to.x() < from.x() {
            return self.state.movable_directions[Direction::West as usize].is_set(from);
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
        pattern.iter().find_map(|&original| {

            // Don't check variants outside the board
            if original.x() > pos.x() || original.y() > pos.y() {
                return None;
            }

            Board::check_variant(board, pattern, pos - original)
        })
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
    fn trickle_column(&mut self, x: u8) -> Vec<(Pos, Pos)> {
        let mut empty_spaces = VecDeque::new();
        let mut moves = Vec::new();

        for y in 0..self.state.height {
            let current_pos = Pos::new(x, y);
            if self.state.empties.is_set(current_pos) {
                empty_spaces.push_back(y);
            } else if self.state.movable_directions[Direction::South as usize].is_set(current_pos) {
                if let Some(space_to_fill) = empty_spaces.pop_front() {
                    self.swap_always(current_pos, Pos::new(x, space_to_fill));
                    empty_spaces.push_back(y);
                    moves.push((Pos::new(x, y), Pos::new(x, space_to_fill)));
                }
            } else {
                empty_spaces.clear();
            }
        }

        moves
    }

    /// Moves all pieces in the board diagonally and down until they can no longer be moved.
    /// Should be called after [trickle_column()](Board::trickle_column) is run on all columns.
    fn trickle_diagonally(&mut self) -> Vec<(Pos, Pos)> {
        let mut moves = Vec::new();

        for y in 0..self.state.height {
            for x in 0..self.state.width {
                moves.append(&mut self.trickle_piece(Pos::new(x, y), true));
            }
        }

        moves
    }

    /// Moves a piece down and diagonally until it can no longer be moved.
    /// Returns the list of swaps that makes the piece fall naturally. See
    /// [trickle()] for a complete list of rules on how the piece is
    /// trickled.
    ///
    /// # Arguments
    ///
    /// * `piece_pos` - the position of the piece to trickle
    /// * `check_adj` - whether to check if the horizontally adjacent piece
    ///                 will fall to fill the spot when all pieces in the row
    ///                 are trickled
    fn trickle_piece(&mut self, piece_pos: Pos, check_adj: bool) -> Vec<(Pos, Pos)> {
        let mut moves = Vec::new();

        if self.state.empties.is_set(piece_pos) {
            return moves;
        }

        let mut previous_trickled_pos;
        let mut current_trickled_pos = piece_pos;

        loop {
            previous_trickled_pos = current_trickled_pos;
            current_trickled_pos = self.trickle_piece_down(previous_trickled_pos);
            if previous_trickled_pos != current_trickled_pos {
                moves.push((previous_trickled_pos, current_trickled_pos));
            }

            previous_trickled_pos = current_trickled_pos;
            current_trickled_pos = self.trickle_piece_diagonally(
                previous_trickled_pos,
                check_adj
            );

            if previous_trickled_pos == current_trickled_pos {
                break;
            } else {
                moves.push((previous_trickled_pos, current_trickled_pos));
            }
        }

        moves
    }

    /// Moves a piece diagonally (down and horizontally). If the space to the left
    /// is open, then the piece moves down and to the left. Otherwise, it moves down
    /// and to the right if that space is open. Returns the new position of the piece.
    ///
    /// # Arguments
    ///
    /// * `piece_pos` - the current position of the piece
    /// * `check_adj` - whether to check if the horizontally adjacent piece
    ///                 will fall to fill the spot when all pieces in the row
    ///                 are trickled
    fn trickle_piece_diagonally(&mut self, piece_pos: Pos, check_adj: bool) -> Pos {
        let mut diagonally_trickled_pos = self.trickle_piece_to_side(piece_pos, true, check_adj);
        if diagonally_trickled_pos == piece_pos {
            diagonally_trickled_pos = self.trickle_piece_to_side(piece_pos, false, check_adj);
        }

        diagonally_trickled_pos
    }

    /// Moves a piece one space down and one space horizontally if there is an
    /// empty space there. Returns the new position of the piece.
    ///
    /// # Arguments
    ///
    /// * `current_pos` - the current position of the piece to move
    /// * `to_west` - whether to move the piece west (or east if false)
    /// * `check_adj` - whether to check if the horizontally adjacent piece
    ///                 will fall to fill the spot when all pieces in the row
    ///                 are trickled
    fn trickle_piece_to_side(&mut self, current_pos: Pos, to_west: bool, check_adj: bool) -> Pos {
        if !self.can_move_pos_down_diagonally(current_pos, to_west) {
            return current_pos;
        }

        let empty_pos = Board::move_pos_down_diagonally(current_pos, to_west);
        let is_empty_pos = self.state.empties.is_set(empty_pos);

        let horizontal_dir_board = match to_west {
            true => &self.state.movable_directions[Direction::West as usize],
            false => &self.state.movable_directions[Direction::East as usize]
        };
        let vertical_dir_board = &self.state.movable_directions[Direction::South as usize];
        let is_movable = horizontal_dir_board.is_set(current_pos) &&
            vertical_dir_board.is_set(current_pos);

        let adjacent_pos = Pos::new(empty_pos.x(), current_pos.y());
        let will_adj_fill_space = check_adj && vertical_dir_board.is_set(adjacent_pos)
            && !self.state.empties.is_set(adjacent_pos);

        if !is_empty_pos || !is_movable || will_adj_fill_space {
            return current_pos;
        }

        self.swap_always(current_pos, empty_pos);

        empty_pos
    }

    /// Moves a piece down until it is moved into the lowest empty space directly
    /// below it. Returns the new position of the piece
    ///
    /// # Arguments
    ///
    /// * `piece_pos` - the current position of the piece to move
    fn trickle_piece_down(&mut self, piece_pos: Pos) -> Pos {
        let vertical_dir_board = &self.state.movable_directions[Direction::South as usize];
        if !vertical_dir_board.is_set(piece_pos){
            return piece_pos;
        }

        let mut next_y = piece_pos.y();
        while next_y > 0 && self.state.empties.is_set(Pos::new(piece_pos.x(), next_y - 1)) {
            next_y -= 1;
        }
        self.swap_always(piece_pos, Pos::new(piece_pos.x(), next_y));

        Pos::new(piece_pos.x(), next_y)
    }

    /// Swaps two pieces regardless of the swap rules. Pieces more than one
    /// space apart can be swapped. Always successful. Marks both spaces
    /// for a match check if they are different.
    ///
    /// # Arguments
    ///
    /// * `first` - the position of a piece to swap
    /// * `second` - the position of another piece to swap
    fn swap_always(&mut self, first: Pos, second: Pos) {
        if first == second {
            return;
        }

        self.state.last_changed.push_back(first);
        self.state.last_changed.push_back(second);

        self.state.empties.swap(first, second);
        self.state.movable_directions[0].swap(first, second);
        self.state.movable_directions[1].swap(first, second);
        self.state.movable_directions[2].swap(first, second);
        self.state.movable_directions[3].swap(first, second);

        let possible_first_type = self.piece_type(first);
        let possible_second_type = self.piece_type(second);

        // We don't want to undo the swap if both pieces are of the same type
        if possible_first_type != possible_second_type {
            if let Some(first_type) = possible_first_type {
                self.state.pieces.entry(first_type).and_modify(
                    |board| board.swap(first, second)
                );
            }

            if let Some(second_type) = possible_second_type {
                self.state.pieces.entry(second_type).and_modify(
                    |board| board.swap(first, second)
                );
            }
        }
    }

    /// Checks if a position can move one space down and one space horizontally
    /// and still be inside the board.
    ///
    /// # Arguments
    ///
    /// * `pos` - the position to move
    /// * `to_west` - whether to move the position west (or east if false)
    fn can_move_pos_down_diagonally(&self, pos: Pos, to_west: bool) -> bool {
        match to_west {
            true => pos.x() > 0 && pos.y() > 0,
            false => pos.x() < self.state.width - 1 && pos.y() > 0
        }
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
        pos.x() < self.state.width && pos.y() < self.state.height
    }

}

impl Debug for Board {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("")
            .field(&self.patterns)
            .field(&self.state)
            .finish()
    }
}

impl Display for Board {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        let mut str = String::new();

        for y in (0..self.state.height).rev() {
            for x in 0..self.state.width {
                str.push_str(&self.piece(Pos::new(x, y)).to_string());
            }

            str.push('\n');
        }

        write!(f, "{}", str)
    }
}

#[cfg(test)]
mod tests {
    use crate::board::{Board, BoardState};
    use crate::position::Pos;
    use crate::piece::{Piece, Direction, ALL_DIRECTIONS};
    use std::collections::{HashSet};
    use crate::matching::MatchPattern;
    use enumset::{enum_set};
    use std::panic;

    #[test]
    #[should_panic]
    fn get_piece_out_of_bounds_panics() {
        let board = Board::new(BoardState::new(16, 16), Vec::new(), vec![
            Box::new(|_, _, _| true),
            Box::new(|_, _, _| true)
        ]);
        board.piece(Pos::new(16, 16));
    }

    #[test]
    fn swap_adjacent_all_rules_passed_swapped() {
        let mut board = Board::new(BoardState::new(16, 16), Vec::new(), vec![
            Box::new(|_, _, _| true),
            Box::new(|_, _, _| true)
        ]);
        let type1 = 'f';
        let type2 = 's';
        let piece1 = Piece::Regular(type1, ALL_DIRECTIONS);
        let piece2 = Piece::Regular(type2, ALL_DIRECTIONS);

        board.set_piece(Pos::new(1, 2), piece1);
        board.set_piece(Pos::new(1, 3), piece2);

        assert!(board.swap_pieces(Pos::new(1, 2), Pos::new(1, 3)));
        match board.piece(Pos::new(1, 2)) {
            Piece::Regular(piece_type, _) => assert_eq!(type2, piece_type),
            _ => panic!("Wrong piece")
        };
        match board.piece(Pos::new(1, 3)) {
            Piece::Regular(piece_type, _) => assert_eq!(type1, piece_type),
            _ => panic!("Wrong piece")
        };
    }

    #[test]
    fn swap_non_adjacent_all_rules_passed_swapped() {
        let mut board = Board::new(BoardState::new(16, 16), Vec::new(), vec![
            Box::new(|_, _, _| true),
            Box::new(|_, _, _| true)
        ]);
        let type1 = 'f';
        let type2 = 's';
        let piece1 = Piece::Regular(type1, ALL_DIRECTIONS);
        let piece2 = Piece::Regular(type2, ALL_DIRECTIONS);

        board.set_piece(Pos::new(1, 2), piece1);
        board.set_piece(Pos::new(14, 15), piece2);

        assert!(board.swap_pieces(Pos::new(1, 2), Pos::new(14, 15)));
        match board.piece(Pos::new(1, 2)) {
            Piece::Regular(piece_type, _) => assert_eq!(type2, piece_type),
            _ => panic!("Wrong piece")
        };
        match board.piece(Pos::new(14, 15)) {
            Piece::Regular(piece_type, _) => assert_eq!(type1, piece_type),
            _ => panic!("Wrong piece")
        };
    }

    #[test]
    fn swap_rules_violated_not_swapped() {
        let mut board = Board::new(BoardState::new(16, 16), Vec::new(), vec![
            Box::new(|_, _, _| true),
            Box::new(|_, _, _| false)
        ]);
        let type1 = 'f';
        let type2 = 's';
        let piece1 = Piece::Regular(type1, ALL_DIRECTIONS);
        let piece2 = Piece::Regular(type2, ALL_DIRECTIONS);

        board.set_piece(Pos::new(1, 2), piece1);
        board.set_piece(Pos::new(1, 3), piece2);

        assert!(!board.swap_pieces(Pos::new(1, 2), Pos::new(1, 3)));
        match board.piece(Pos::new(1, 2)) {
            Piece::Regular(piece_type, _) => assert_eq!(type1, piece_type),
            _ => panic!("Wrong piece")
        };
        match board.piece(Pos::new(1, 3)) {
            Piece::Regular(piece_type, _) => assert_eq!(type2, piece_type),
            _ => panic!("Wrong piece")
        };
    }

    #[test]
    fn swap_rules_violated_short_circuits() {
        let mut board = Board::new(BoardState::new(16, 16), Vec::new(), vec![
            Box::new(|_, _, _| false),
            Box::new(|_, _, _| { panic!("Should short circuit before this") })
        ]);
        let type1 = 'f';
        let type2 = 's';
        let piece1 = Piece::Regular(type1, ALL_DIRECTIONS);
        let piece2 = Piece::Regular(type2, ALL_DIRECTIONS);

        board.set_piece(Pos::new(1, 2), piece1);
        board.set_piece(Pos::new(1, 3), piece2);

        assert!(!board.swap_pieces(Pos::new(1, 2), Pos::new(1, 3)));
    }

    #[test]
    fn swap_empty_all_rules_passed_swapped() {
        let mut board = Board::new(BoardState::new(16, 16), Vec::new(), vec![
            Box::new(|_, _, _| true),
            Box::new(|_, _, _| true)
        ]);
        let type1 = 'f';
        let piece1 = Piece::Regular(type1, ALL_DIRECTIONS);

        board.set_piece(Pos::new(1, 2), piece1);
        board.set_piece(Pos::new(1, 3), Piece::Empty);

        assert!(board.swap_pieces(Pos::new(1, 2), Pos::new(1, 3)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(1, 2)));
        match board.piece(Pos::new(1, 3)) {
            Piece::Regular(piece_type, _) => assert_eq!(type1, piece_type),
            _ => panic!("Wrong piece")
        };
    }

    #[test]
    fn swap_wall_all_rules_passed_not_swapped() {
        let mut board = Board::new(BoardState::new(16, 16), Vec::new(), vec![
            Box::new(|_, _, _| true),
            Box::new(|_, _, _| true)
        ]);
        let type1 = 'f';
        let piece1 = Piece::Regular(type1, ALL_DIRECTIONS);

        board.set_piece(Pos::new(1, 2), piece1);

        assert!(!board.swap_pieces(Pos::new(1, 2), Pos::new(1, 3)));
        assert_eq!(Piece::Wall, board.piece(Pos::new(1, 3)));
        match board.piece(Pos::new(1, 2)) {
            Piece::Regular(piece_type, _) => assert_eq!(type1, piece_type),
            _ => panic!("Wrong piece")
        };
    }

    #[test]
    #[should_panic]
    #[allow(unused_must_use)]
    fn swap_first_pos_outside_board_panics() {
        let mut board = Board::new(BoardState::new(16, 16), Vec::new(), vec![
            Box::new(|_, _, _| true),
            Box::new(|_, _, _| true)
        ]);
        let type1 = 'f';
        let piece1 = Piece::Regular(type1, ALL_DIRECTIONS);

        board.set_piece(Pos::new(1, 2), piece1);

        board.swap_pieces(Pos::new(16, 16), Pos::new(1, 2));
    }

    #[test]
    #[should_panic]
    #[allow(unused_must_use)]
    fn swap_first_pos_very_large_panics() {
        let mut board = Board::new(BoardState::new(16, 16), Vec::new(), vec![
            Box::new(|_, _, _| true),
            Box::new(|_, _, _| true)
        ]);
        let type1 = 'f';
        let piece1 = Piece::Regular(type1, ALL_DIRECTIONS);

        board.set_piece(Pos::new(1, 2), piece1);

        board.swap_pieces(Pos::new(u8::MAX, u8::MAX), Pos::new(1, 2));
    }

    #[test]
    #[should_panic]
    #[allow(unused_must_use)]
    fn swap_second_pos_outside_board_panics() {
        let mut board = Board::new(BoardState::new(16, 16), Vec::new(), vec![
            Box::new(|_, _, _| true),
            Box::new(|_, _, _| true)
        ]);
        let type1 = 'f';
        let piece1 = Piece::Regular(type1, ALL_DIRECTIONS);

        board.set_piece(Pos::new(1, 2), piece1);

        board.swap_pieces(Pos::new(1, 2), Pos::new(16, 16));
    }

    #[test]
    #[should_panic]
    #[allow(unused_must_use)]
    fn swap_second_pos_very_large_panics() {
        let mut board = Board::new(BoardState::new(16, 16), Vec::new(), vec![
            Box::new(|_, _, _| true),
            Box::new(|_, _, _| true)
        ]);
        let type1 = 'f';
        let piece1 = Piece::Regular(type1, ALL_DIRECTIONS);

        board.set_piece(Pos::new(1, 2), piece1);

        board.swap_pieces(Pos::new(1, 2), Pos::new(u8::MAX, u8::MAX));
    }

    #[test]
    fn swap_self_all_rules_passed_swapped() {
        let mut board = Board::new(BoardState::new(16, 16), Vec::new(), vec![
            Box::new(|_, _, _| true),
            Box::new(|_, _, _| true)
        ]);
        let type1 = 'f';
        let piece1 = Piece::Regular(type1, ALL_DIRECTIONS);

        board.set_piece(Pos::new(1, 2), piece1);

        assert!(board.swap_pieces(Pos::new(1, 2), Pos::new(1, 2)));
        match board.piece(Pos::new(1, 2)) {
            Piece::Regular(piece_type, _) => assert_eq!(type1, piece_type),
            _ => panic!("Wrong piece")
        };
    }

    #[test]
    fn swap_same_vertical_not_vertically_movable_swapped() {
        let mut board = Board::new(BoardState::new(16, 16), Vec::new(), vec![
            Box::new(|_, _, _| true),
            Box::new(|_, _, _| true)
        ]);
        let type1 = 'f';
        let type2 = 's';
        let piece1 = Piece::Regular(type1, enum_set!(Direction::West | Direction::East));

        let piece2 = Piece::Regular(type2, ALL_DIRECTIONS);

        board.set_piece(Pos::new(1, 2), piece1);
        board.set_piece(Pos::new(2, 2), piece2);

        assert!(board.swap_pieces(Pos::new(1, 2), Pos::new(2, 2)));
        match board.piece(Pos::new(1, 2)) {
            Piece::Regular(piece_type, _) => assert_eq!(type2, piece_type),
            _ => panic!("Wrong piece")
        };
        match board.piece(Pos::new(2, 2)) {
            Piece::Regular(piece_type, _) => assert_eq!(type1, piece_type),
            _ => panic!("Wrong piece")
        };
    }

    #[test]
    fn swap_same_horizontal_not_horizontally_movable_swapped() {
        let mut board = Board::new(BoardState::new(16, 16), Vec::new(), vec![
            Box::new(|_, _, _| true),
            Box::new(|_, _, _| true)
        ]);
        let type1 = 'f';
        let type2 = 's';
        let piece1 = Piece::Regular(type1, enum_set!(Direction::North | Direction::South));

        let piece2 = Piece::Regular(type2, ALL_DIRECTIONS);

        board.set_piece(Pos::new(1, 2), piece1);
        board.set_piece(Pos::new(1, 3), piece2);

        assert!(board.swap_pieces(Pos::new(1, 2), Pos::new(1, 3)));
        match board.piece(Pos::new(1, 2)) {
            Piece::Regular(piece_type, _) => assert_eq!(type2, piece_type),
            _ => panic!("Wrong piece")
        };
        match board.piece(Pos::new(1, 3)) {
            Piece::Regular(piece_type, _) => assert_eq!(type1, piece_type),
            _ => panic!("Wrong piece")
        };
    }

    #[test]
    fn swap_north_not_movable_north_not_swapped() {
        let mut board = Board::new(BoardState::new(16, 16), Vec::new(), vec![
            Box::new(|_, _, _| true),
            Box::new(|_, _, _| true)
        ]);
        let type1 = 'f';
        let type2 = 's';
        let piece1 = Piece::Regular(type1, enum_set!(
            Direction::South | Direction::East | Direction::West
        ));

        let piece2 = Piece::Regular(type2, ALL_DIRECTIONS);

        board.set_piece(Pos::new(1, 2), piece1);
        board.set_piece(Pos::new(1, 3), piece2);

        assert!(!board.swap_pieces(Pos::new(1, 2), Pos::new(1, 3)));

        match board.piece(Pos::new(1, 2)) {
            Piece::Regular(piece_type, _) => assert_eq!(type1, piece_type),
            _ => panic!("Wrong piece")
        };
        match board.piece(Pos::new(1, 3)) {
            Piece::Regular(piece_type, _) => assert_eq!(type2, piece_type),
            _ => panic!("Wrong piece")
        };
    }

    #[test]
    fn swap_south_not_movable_south_not_swapped() {
        let mut board = Board::new(BoardState::new(16, 16), Vec::new(), vec![
            Box::new(|_, _, _| true),
            Box::new(|_, _, _| true)
        ]);
        let type1 = 'f';
        let type2 = 's';
        let piece1 = Piece::Regular(type1, enum_set!(
            Direction::North | Direction::East | Direction::West
        ));

        let piece2 = Piece::Regular(type2, ALL_DIRECTIONS);

        board.set_piece(Pos::new(1, 2), piece1);
        board.set_piece(Pos::new(1, 0), piece2);

        assert!(!board.swap_pieces(Pos::new(1, 2), Pos::new(1, 5)));
        match board.piece(Pos::new(1, 2)) {
            Piece::Regular(piece_type, _) => assert_eq!(type1, piece_type),
            _ => panic!("Wrong piece")
        };
        match board.piece(Pos::new(1, 0)) {
            Piece::Regular(piece_type, _) => assert_eq!(type2, piece_type),
            _ => panic!("Wrong piece")
        };
    }

    #[test]
    fn swap_east_not_movable_east_not_swapped() {
        let mut board = Board::new(BoardState::new(16, 16), Vec::new(), vec![
            Box::new(|_, _, _| true),
            Box::new(|_, _, _| true)
        ]);
        let type1 = 'f';
        let type2 = 's';
        let piece1 = Piece::Regular(type1, enum_set!(
            Direction::North | Direction::South | Direction::West
        ));

        let piece2 = Piece::Regular(type2, ALL_DIRECTIONS);

        board.set_piece(Pos::new(1, 2), piece1);
        board.set_piece(Pos::new(2, 3), piece2);

        assert!(!board.swap_pieces(Pos::new(1, 2), Pos::new(2, 3)));
        match board.piece(Pos::new(1, 2)) {
            Piece::Regular(piece_type, _) => assert_eq!(type1, piece_type),
            _ => panic!("Wrong piece")
        };
        match board.piece(Pos::new(2, 3)) {
            Piece::Regular(piece_type, _) => assert_eq!(type2, piece_type),
            _ => panic!("Wrong piece")
        };
    }

    #[test]
    fn swap_west_not_movable_west_not_swapped() {
        let mut board = Board::new(BoardState::new(16, 16), Vec::new(), vec![
            Box::new(|_, _, _| true),
            Box::new(|_, _, _| true)
        ]);
        let type1 = 'f';
        let type2 = 's';
        let piece1 = Piece::Regular(type1, enum_set!(
            Direction::North | Direction::South | Direction::East
        ));

        let piece2 = Piece::Regular(type2, ALL_DIRECTIONS);

        board.set_piece(Pos::new(1, 2), piece1);
        board.set_piece(Pos::new(0, 2), piece2);

        assert!(!board.swap_pieces(Pos::new(1, 2), Pos::new(4, 3)));

        match board.piece(Pos::new(1, 2)) {
            Piece::Regular(piece_type, _) => assert_eq!(type1, piece_type),
            _ => panic!("Wrong piece")
        };
        match board.piece(Pos::new(0, 2)) {
            Piece::Regular(piece_type, _) => assert_eq!(type2, piece_type),
            _ => panic!("Wrong piece")
        };
    }

    #[test]
    fn set_piece_not_present_wall_returned() {
        let mut board = Board::new(BoardState::new(16, 16), Vec::new(), vec![
            Box::new(|_, _, _| true),
            Box::new(|_, _, _| true)
        ]);
        let type1 = 'f';
        let piece1 = Piece::Regular(type1, ALL_DIRECTIONS);

        assert_eq!(Piece::Wall, board.set_piece(Pos::new(1, 2), piece1));

        match board.piece(Pos::new(1, 2)) {
            Piece::Regular(piece_type, _) => assert_eq!(type1, piece_type),
            _ => panic!("Wrong piece")
        };
    }

    #[test]
    fn set_piece_wall_old_returned() {
        let mut board = Board::new(BoardState::new(16, 16), Vec::new(), vec![
            Box::new(|_, _, _| true),
            Box::new(|_, _, _| true)
        ]);
        let type1 = 'f';
        let piece1 = Piece::Regular(type1, ALL_DIRECTIONS);

        board.set_piece(Pos::new(1, 2), piece1);

        match board.set_piece(Pos::new(1, 2), Piece::Wall) {
            Piece::Regular(piece_type, _) => assert_eq!(type1, piece_type),
            _ => panic!("Wrong piece")
        };
        assert_eq!(Piece::Wall, board.piece(Pos::new(1, 2)));
    }

    #[test]
    fn set_piece_empty_old_returned() {
        let mut board = Board::new(BoardState::new(16, 16), Vec::new(), vec![
            Box::new(|_, _, _| true),
            Box::new(|_, _, _| true)
        ]);
        let type1 = 'f';
        let piece1 = Piece::Regular(type1, ALL_DIRECTIONS);

        board.set_piece(Pos::new(1, 2), piece1);

        match board.set_piece(Pos::new(1, 2), Piece::Empty) {
            Piece::Regular(piece_type, _) => assert_eq!(type1, piece_type),
            _ => panic!("Wrong piece")
        };
        assert_eq!(Piece::Empty, board.piece(Pos::new(1, 2)));
    }

    #[test]
    fn set_piece_duplicate_old_returned() {
        let mut board = Board::new(BoardState::new(16, 16), Vec::new(), vec![
            Box::new(|_, _, _| true),
            Box::new(|_, _, _| true)
        ]);
        let type1 = 'f';
        let piece1 = Piece::Regular(type1, ALL_DIRECTIONS);

        board.set_piece(Pos::new(1, 2), piece1);

        assert_eq!(piece1, board.set_piece(Pos::new(1, 2), piece1));
        match board.piece(Pos::new(1, 2)) {
            Piece::Regular(piece_type, _) => assert_eq!(type1, piece_type),
            _ => panic!("Wrong piece")
        };
    }

    #[test]
    fn set_piece_present_old_piece_returned() {
        let mut board = Board::new(BoardState::new(16, 16), Vec::new(), vec![
            Box::new(|_, _, _| true),
            Box::new(|_, _, _| true)
        ]);
        let type1 = 'f';
        let type2 = 's';
        let piece1 = Piece::Regular(type1, ALL_DIRECTIONS);
        let piece2 = Piece::Regular(type2, ALL_DIRECTIONS);

        board.set_piece(Pos::new(1, 2), piece1);
        match board.set_piece(Pos::new(1, 2), piece2) {
            Piece::Regular(piece_type, _) => assert_eq!(type1, piece_type),
            _ => panic!("Wrong piece")
        };
    }

    #[test]
    #[should_panic]
    fn set_piece_out_of_bounds_panics() {
        let mut board = Board::new(BoardState::new(16, 16), Vec::new(), vec![
            Box::new(|_, _, _| true),
            Box::new(|_, _, _| true)
        ]);
        let type1 = 'f';
        let piece1 = Piece::Regular(type1, ALL_DIRECTIONS);

        board.set_piece(Pos::new(16, 16), piece1);
    }

    #[test]
    fn next_match_no_patterns_none() {
        let mut board = Board::new(BoardState::new(16, 16), Vec::new(), vec![
            Box::new(|_, _, _| true),
            Box::new(|_, _, _| true)
        ]);
        let type1 = 'f';
        let piece1 = Piece::Regular(type1, ALL_DIRECTIONS);
        let piece2 = Piece::Regular(type1, ALL_DIRECTIONS);

        board.set_piece(Pos::new(1, 2), piece1);
        board.set_piece(Pos::new(2, 3), piece2);
        assert!(board.next_match().is_none());
    }

    #[test]
    fn next_match_set_pieces_match_found() {
        let mut pattern_pos = HashSet::new();
        pattern_pos.insert(Pos::new(2, 3));
        pattern_pos.insert(Pos::new(3, 3));
        pattern_pos.insert(Pos::new(6, 8));

        let type1 = 'f';

        let mut board = Board::new(
            BoardState::new(16, 16),
            vec![MatchPattern::new(type1, pattern_pos, 1)],
            Vec::new()
        );
        let piece1 = Piece::Regular(type1, ALL_DIRECTIONS);
        let piece2 = Piece::Regular(type1, ALL_DIRECTIONS);
        let piece3 = Piece::Regular(type1, ALL_DIRECTIONS);

        board.set_piece(Pos::new(0, 1), piece1);
        board.set_piece(Pos::new(1, 1), piece2);
        board.set_piece(Pos::new(4, 6), piece3);

        let next_match = board.next_match().unwrap();
        assert_eq!(Pos::new(0, 1), next_match.changed_pos());
        assert!(next_match.board_pos().contains(&Pos::new(0, 1)));
        assert!(next_match.board_pos().contains(&Pos::new(1, 1)));
        assert!(next_match.board_pos().contains(&Pos::new(4, 6)));
    }

    #[test]
    fn next_match_swap_pieces_match_found() {
        let mut pattern_pos = HashSet::new();
        pattern_pos.insert(Pos::new(2, 3));
        pattern_pos.insert(Pos::new(3, 3));
        pattern_pos.insert(Pos::new(8, 8));

        let type1 = 'f';

        let mut board = Board::new(
            BoardState::new(16, 16),
            vec![MatchPattern::new(type1, pattern_pos, 1)],
            Vec::new()
        );
        let piece1 = Piece::Regular(type1, ALL_DIRECTIONS);
        let piece2 = Piece::Regular(type1, ALL_DIRECTIONS);
        let piece3 = Piece::Regular(type1, ALL_DIRECTIONS);

        board.set_piece(Pos::new(0, 1), piece1);
        board.set_piece(Pos::new(1, 1), piece2);
        board.set_piece(Pos::new(8, 8), piece3);
        board.set_piece(Pos::new(6, 6), Piece::Empty);
        board.next_match();
        board.next_match();
        board.next_match();
        board.next_match();

        assert!(board.swap_pieces(Pos::new(6, 6), Pos::new(8, 8)));

        let next_match = board.next_match().unwrap();
        assert_eq!(Pos::new(6, 6), next_match.changed_pos());
        assert!(next_match.board_pos().contains(&Pos::new(0, 1)));
        assert!(next_match.board_pos().contains(&Pos::new(1, 1)));
        assert!(next_match.board_pos().contains(&Pos::new(6, 6)));
    }

    #[test]
    fn next_match_swap_self_no_match_found() {
        let mut pattern_pos = HashSet::new();
        pattern_pos.insert(Pos::new(2, 3));
        pattern_pos.insert(Pos::new(3, 3));
        pattern_pos.insert(Pos::new(8, 8));

        let type1 = 'f';

        let mut board = Board::new(
            BoardState::new(16, 16),
            vec![MatchPattern::new(type1, pattern_pos, 1)],
            Vec::new()
        );
        let piece1 = Piece::Regular(type1, ALL_DIRECTIONS);
        let piece2 = Piece::Regular(type1, ALL_DIRECTIONS);
        let piece3 = Piece::Regular(type1, ALL_DIRECTIONS);

        board.set_piece(Pos::new(0, 1), piece1);
        board.set_piece(Pos::new(1, 1), piece2);
        board.set_piece(Pos::new(6, 6), piece3);
        board.next_match();
        board.next_match();
        board.next_match();

        assert!(board.swap_pieces(Pos::new(6, 6), Pos::new(6, 6)));
        assert!(board.next_match().is_none());
    }

    #[test]
    fn next_match_trickle_match_found() {
        let type1 = 'f';

        let mut pattern_pos1 = HashSet::new();
        pattern_pos1.insert(Pos::new(0, 0));
        pattern_pos1.insert(Pos::new(0, 1));
        pattern_pos1.insert(Pos::new(1, 0));

        let mut board = Board::new(BoardState::new(16, 16), vec![
            MatchPattern::new(type1, pattern_pos1, 1)
        ], Vec::new());
        let piece1 = Piece::Regular(type1, ALL_DIRECTIONS);

        board.set_piece(Pos::new(0, 0), piece1);
        board.set_piece(Pos::new(0, 1), piece1);
        board.set_piece(Pos::new(0, 2), piece1);
        board.set_piece(Pos::new(1, 0), Piece::Empty);
        board.set_piece(Pos::new(1, 1), Piece::Empty);
        board.set_piece(Pos::new(1, 2), Piece::Empty);

        for _ in 0..6 {
            board.next_match();
        }

        board.trickle();

        let next_match = board.next_match().unwrap();
        assert_eq!(Pos::new(0, 1), next_match.changed_pos());
        assert!(next_match.board_pos().contains(&Pos::new(0, 0)));
        assert!(next_match.board_pos().contains(&Pos::new(1, 0)));
        assert!(next_match.board_pos().contains(&Pos::new(0, 1)));
    }

    #[test]
    fn next_match_add_trickle_match_found() {
        let type1 = 'f';

        let mut pattern_pos1 = HashSet::new();
        pattern_pos1.insert(Pos::new(0, 0));
        pattern_pos1.insert(Pos::new(0, 1));
        pattern_pos1.insert(Pos::new(1, 0));

        let mut board = Board::new(BoardState::new(16, 16), vec![
            MatchPattern::new(type1, pattern_pos1, 1)
        ], Vec::new());
        let piece1 = Piece::Regular(type1, ALL_DIRECTIONS);

        board.set_piece(Pos::new(0, 0), piece1);
        board.set_piece(Pos::new(0, 1), piece1);
        board.set_piece(Pos::new(0, 2), Piece::Empty);
        board.set_piece(Pos::new(1, 0), Piece::Empty);
        board.set_piece(Pos::new(1, 1), Piece::Empty);
        board.set_piece(Pos::new(1, 2), Piece::Empty);

        for _ in 0..6 {
            board.next_match();
        }

        board.add_and_trickle(Pos::new(0, 2), piece1);

        let next_match = board.next_match().unwrap();
        assert_eq!(Pos::new(1, 0), next_match.changed_pos());
        assert!(next_match.board_pos().contains(&Pos::new(0, 0)));
        assert!(next_match.board_pos().contains(&Pos::new(1, 0)));
        assert!(next_match.board_pos().contains(&Pos::new(0, 1)));
    }

    #[test]
    fn next_match_matches_all_variants() {
        let piece_type = 'f';
        let mut pattern_pos = HashSet::new();
        pattern_pos.insert(Pos::new(2, 2));
        pattern_pos.insert(Pos::new(3, 3));
        pattern_pos.insert(Pos::new(4, 4));

        let type1 = 'f';

        let mut board = Board::new(
            BoardState::new(16, 16),
            vec![MatchPattern::new(type1, pattern_pos, 1)],
            Vec::new()
        );
        let piece1 = Piece::Regular(piece_type, ALL_DIRECTIONS);
        let piece2 = Piece::Regular(piece_type, ALL_DIRECTIONS);
        let piece3 = Piece::Regular(piece_type, ALL_DIRECTIONS);

        board.set_piece(Pos::new(0, 0), piece1);
        board.set_piece(Pos::new(1, 1), piece2);
        board.set_piece(Pos::new(2, 2), piece3);

        let next_match1 = board.next_match().unwrap();
        assert_eq!(Pos::new(0, 0), next_match1.changed_pos());
        assert!(next_match1.board_pos().contains(&Pos::new(0, 0)));
        assert!(next_match1.board_pos().contains(&Pos::new(1, 1)));
        assert!(next_match1.board_pos().contains(&Pos::new(2, 2)));

        let next_match2 = board.next_match().unwrap();
        assert_eq!(Pos::new(1, 1), next_match2.changed_pos());
        assert!(next_match2.board_pos().contains(&Pos::new(0, 0)));
        assert!(next_match2.board_pos().contains(&Pos::new(1, 1)));
        assert!(next_match2.board_pos().contains(&Pos::new(2, 2)));

        let next_match3 = board.next_match().unwrap();
        assert_eq!(Pos::new(2, 2), next_match3.changed_pos());
        assert!(next_match3.board_pos().contains(&Pos::new(0, 0)));
        assert!(next_match3.board_pos().contains(&Pos::new(1, 1)));
        assert!(next_match3.board_pos().contains(&Pos::new(2, 2)));
    }

    #[test]
    fn next_match_does_not_match_wrong_types() {
        let type1 = 'f';
        let type2 = 's';
        let mut pattern_pos = HashSet::new();
        pattern_pos.insert(Pos::new(2, 2));
        pattern_pos.insert(Pos::new(3, 3));
        pattern_pos.insert(Pos::new(4, 4));

        let mut board = Board::new(
            BoardState::new(16, 16),
            vec![MatchPattern::new(type2, pattern_pos, 1)],
            Vec::new()
        );
        let piece1 = Piece::Regular(type1, ALL_DIRECTIONS);
        let piece2 = Piece::Regular(type1, ALL_DIRECTIONS);
        let piece3 = Piece::Regular(type1, ALL_DIRECTIONS);

        board.set_piece(Pos::new(0, 0), piece1);
        board.set_piece(Pos::new(1, 1), piece2);
        board.set_piece(Pos::new(2, 2), piece3);

        assert!(board.next_match().is_none());
        assert!(board.next_match().is_none());
        assert!(board.next_match().is_none());
    }

    #[test]
    fn next_match_matches_when_not_all_in_queue() {
        let type1 = 'f';
        let mut pattern_pos = HashSet::new();
        pattern_pos.insert(Pos::new(2, 2));
        pattern_pos.insert(Pos::new(3, 3));
        pattern_pos.insert(Pos::new(4, 4));

        let mut board = Board::new(
            BoardState::new(16, 16),
            vec![MatchPattern::new(type1, pattern_pos, 1)],
            Vec::new()
        );
        let piece1 = Piece::Regular(type1, ALL_DIRECTIONS);
        let piece2 = Piece::Regular(type1, ALL_DIRECTIONS);
        let piece3 = Piece::Regular(type1, ALL_DIRECTIONS);

        board.set_piece(Pos::new(0, 0), piece1);
        board.set_piece(Pos::new(1, 1), piece2);

        board.next_match();
        board.next_match();

        board.set_piece(Pos::new(2, 2), piece3);

        let next_match = board.next_match().unwrap();
        assert_eq!(Pos::new(2, 2), next_match.changed_pos());
        assert!(next_match.board_pos().contains(&Pos::new(0, 0)));
        assert!(next_match.board_pos().contains(&Pos::new(1, 1)));
        assert!(next_match.board_pos().contains(&Pos::new(2, 2)));
    }

    #[test]
    fn next_match_board_state_changed_after_match_still_matches() {
        let type1 = 'f';
        let mut pattern_pos = HashSet::new();
        pattern_pos.insert(Pos::new(2, 2));
        pattern_pos.insert(Pos::new(3, 3));
        pattern_pos.insert(Pos::new(4, 4));

        let mut board = Board::new(
            BoardState::new(16, 16),
            vec![MatchPattern::new(type1, pattern_pos, 1)],
            Vec::new()
        );
        let piece1 = Piece::Regular(type1, ALL_DIRECTIONS);
        let piece2 = Piece::Regular(type1, ALL_DIRECTIONS);
        let piece3 = Piece::Regular(type1, ALL_DIRECTIONS);
        let piece4 = Piece::Regular(type1, ALL_DIRECTIONS);

        board.set_piece(Pos::new(0, 0), piece1);
        board.set_piece(Pos::new(1, 1), piece2);

        board.next_match();
        board.next_match();

        board.set_piece(Pos::new(2, 2), piece3);
        board.set_piece(Pos::new(2, 3), piece4);

        let next_match = board.next_match().unwrap();
        assert_eq!(Pos::new(2, 2), next_match.changed_pos());
        assert!(next_match.board_pos().contains(&Pos::new(0, 0)));
        assert!(next_match.board_pos().contains(&Pos::new(1, 1)));
        assert!(next_match.board_pos().contains(&Pos::new(2, 2)));
    }

    #[test]
    fn next_match_match_overwritten_does_not_match() {
        let type1 = 'f';
        let mut pattern_pos = HashSet::new();
        pattern_pos.insert(Pos::new(2, 2));
        pattern_pos.insert(Pos::new(3, 3));
        pattern_pos.insert(Pos::new(4, 4));

        let mut board = Board::new(
            BoardState::new(16, 16),
            vec![MatchPattern::new(type1, pattern_pos, 1)],
            Vec::new()
        );
        let piece1 = Piece::Regular(type1, ALL_DIRECTIONS);
        let piece2 = Piece::Regular(type1, ALL_DIRECTIONS);
        let piece3 = Piece::Regular(type1, ALL_DIRECTIONS);

        board.set_piece(Pos::new(0, 0), piece1);
        board.set_piece(Pos::new(1, 1), piece2);
        board.set_piece(Pos::new(2, 3), Piece::Empty);

        board.next_match();
        board.next_match();
        board.next_match();

        board.set_piece(Pos::new(2, 2), piece3);
        assert!(board.swap_pieces(Pos::new(2, 2), Pos::new(2, 3)));
        assert!(board.next_match().is_none());
    }

    #[test]
    fn next_match_position_in_queue_twice_matches_twice() {
        let piece_type = 'f';
        let mut pattern_pos = HashSet::new();
        pattern_pos.insert(Pos::new(2, 2));
        pattern_pos.insert(Pos::new(3, 3));
        pattern_pos.insert(Pos::new(4, 4));

        let mut board = Board::new(
            BoardState::new(16, 16),
            vec![MatchPattern::new(piece_type, pattern_pos, 1)],
            Vec::new()
        );
        let piece1 = Piece::Regular(piece_type, ALL_DIRECTIONS);
        let piece2 = Piece::Regular(piece_type, ALL_DIRECTIONS);
        let piece3 = Piece::Regular(piece_type, ALL_DIRECTIONS);
        let piece4 = Piece::Regular(piece_type, ALL_DIRECTIONS);

        board.set_piece(Pos::new(0, 0), piece1);
        board.set_piece(Pos::new(1, 1), piece2);

        board.next_match();
        board.next_match();

        board.set_piece(Pos::new(2, 2), piece3);
        board.set_piece(Pos::new(2, 2), piece4);

        let next_match = board.next_match().unwrap();
        assert_eq!(Pos::new(2, 2), next_match.changed_pos());
        assert!(next_match.board_pos().contains(&Pos::new(0, 0)));
        assert!(next_match.board_pos().contains(&Pos::new(1, 1)));
        assert!(next_match.board_pos().contains(&Pos::new(2, 2)));
    }

    #[test]
    fn next_match_two_patterns_same_rank_matching_picked() {
        let type1 = 'f';
        let type2 = 's';

        let mut pattern_pos1 = HashSet::new();
        pattern_pos1.insert(Pos::new(2, 2));
        pattern_pos1.insert(Pos::new(3, 3));
        pattern_pos1.insert(Pos::new(4, 4));

        let mut pattern_pos2 = HashSet::new();
        pattern_pos2.insert(Pos::new(2, 2));
        pattern_pos2.insert(Pos::new(3, 3));
        pattern_pos2.insert(Pos::new(4, 4));

        let mut board = Board::new(BoardState::new(16, 16), vec![
            MatchPattern::new(type2, pattern_pos1, 1),
            MatchPattern::new(type1, pattern_pos2, 1)
        ], Vec::new());
        let piece1 = Piece::Regular(type1, ALL_DIRECTIONS);
        let piece2 = Piece::Regular(type1, ALL_DIRECTIONS);
        let piece3 = Piece::Regular(type1, ALL_DIRECTIONS);

        board.set_piece(Pos::new(0, 0), piece1);
        board.set_piece(Pos::new(1, 1), piece2);

        board.next_match();
        board.next_match();

        board.set_piece(Pos::new(2, 2), piece3);

        let next_match = board.next_match().unwrap();
        assert_eq!(Pos::new(2, 2), next_match.changed_pos());
        assert!(next_match.board_pos().contains(&Pos::new(0, 0)));
        assert!(next_match.board_pos().contains(&Pos::new(1, 1)));
        assert!(next_match.board_pos().contains(&Pos::new(2, 2)));
    }

    #[test]
    fn next_match_two_patterns_different_rank_higher_picked() {
        let type1 = 'f';

        let mut pattern_pos1 = HashSet::new();
        pattern_pos1.insert(Pos::new(2, 2));
        pattern_pos1.insert(Pos::new(3, 3));
        pattern_pos1.insert(Pos::new(4, 4));

        let mut pattern_pos2 = HashSet::new();
        pattern_pos2.insert(Pos::new(1, 1));
        pattern_pos2.insert(Pos::new(2, 2));
        pattern_pos2.insert(Pos::new(3, 3));
        pattern_pos2.insert(Pos::new(4, 4));

        let mut board = Board::new(BoardState::new(16, 16), vec![
            MatchPattern::new(type1, pattern_pos1, 1),
            MatchPattern::new(type1, pattern_pos2, 2)
        ], Vec::new());
        let piece1 = Piece::Regular(type1, ALL_DIRECTIONS);
        let piece2 = Piece::Regular(type1, ALL_DIRECTIONS);
        let piece3 = Piece::Regular(type1, ALL_DIRECTIONS);
        let piece4 = Piece::Regular(type1, ALL_DIRECTIONS);

        board.set_piece(Pos::new(0, 0), piece1);
        board.set_piece(Pos::new(1, 1), piece2);
        board.set_piece(Pos::new(2, 2), piece3);

        board.next_match();
        board.next_match();
        board.next_match();

        board.set_piece(Pos::new(3, 3), piece4);

        let next_match = board.next_match().unwrap();
        assert_eq!(Pos::new(3, 3), next_match.changed_pos());
        assert!(next_match.board_pos().contains(&Pos::new(0, 0)));
        assert!(next_match.board_pos().contains(&Pos::new(1, 1)));
        assert!(next_match.board_pos().contains(&Pos::new(2, 2)));
        assert!(next_match.board_pos().contains(&Pos::new(3, 3)));
    }

    #[test]
    fn trickle_no_diagonals_sets_board() {
        let type1 = 'f';
        let piece1 = Piece::Regular(type1, ALL_DIRECTIONS);

        let mut board = Board::new(BoardState::new(16, 16), 
                                   Vec::new(), Vec::new());

        board.set_piece(Pos::new(0, 0), piece1);
        board.set_piece(Pos::new(0, 1), Piece::Empty);
        board.set_piece(Pos::new(0, 2), Piece::Empty);
        board.set_piece(Pos::new(0, 3), Piece::Empty);
        board.set_piece(Pos::new(0, 4), Piece::Empty);
        board.set_piece(Pos::new(0, 5), Piece::Empty);

        board.set_piece(Pos::new(1, 0), piece1);
        board.set_piece(Pos::new(1, 1), Piece::Empty);
        board.set_piece(Pos::new(1, 2), piece1);
        board.set_piece(Pos::new(1, 3), Piece::Empty);
        board.set_piece(Pos::new(1, 4), Piece::Empty);
        board.set_piece(Pos::new(1, 5), Piece::Empty);

        board.set_piece(Pos::new(2, 0), Piece::Empty);
        board.set_piece(Pos::new(2, 1), Piece::Empty);
        board.set_piece(Pos::new(2, 2), Piece::Empty);
        board.set_piece(Pos::new(2, 3), piece1);
        board.set_piece(Pos::new(2, 4), Piece::Empty);
        board.set_piece(Pos::new(2, 5), piece1);

        board.set_piece(Pos::new(3, 0), Piece::Empty);
        board.set_piece(Pos::new(3, 1), Piece::Empty);
        board.set_piece(Pos::new(3, 2), Piece::Empty);
        board.set_piece(Pos::new(3, 3), Piece::Empty);
        board.set_piece(Pos::new(3, 4), piece1);
        board.set_piece(Pos::new(3, 5), Piece::Empty);

        board.trickle();

        assert_eq!(piece1, board.piece(Pos::new(0, 0)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(0, 1)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(0, 2)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(0, 3)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(0, 4)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(0, 5)));

        assert_eq!(piece1, board.piece(Pos::new(1, 0)));
        assert_eq!(piece1, board.piece(Pos::new(1, 1)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(1, 2)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(1, 3)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(1, 4)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(1, 5)));

        assert_eq!(piece1, board.piece(Pos::new(2, 0)));
        assert_eq!(piece1, board.piece(Pos::new(2, 1)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(2, 2)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(2, 3)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(2, 4)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(2, 5)));

        assert_eq!(piece1, board.piece(Pos::new(3, 0)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(3, 1)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(3, 2)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(3, 3)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(3, 4)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(3, 5)));
    }

    #[test]
    fn trickle_no_diagonals_generates_moves() {
        let type1 = 'f';
        let piece1 = Piece::Regular(type1, ALL_DIRECTIONS);

        let mut board = Board::new(BoardState::new(16, 16), 
                                   Vec::new(), Vec::new());

        board.set_piece(Pos::new(0, 0), piece1);
        board.set_piece(Pos::new(0, 1), Piece::Empty);
        board.set_piece(Pos::new(0, 2), Piece::Empty);
        board.set_piece(Pos::new(0, 3), Piece::Empty);
        board.set_piece(Pos::new(0, 4), Piece::Empty);
        board.set_piece(Pos::new(0, 5), Piece::Empty);

        board.set_piece(Pos::new(1, 0), piece1);
        board.set_piece(Pos::new(1, 1), Piece::Empty);
        board.set_piece(Pos::new(1, 2), piece1);
        board.set_piece(Pos::new(1, 3), Piece::Empty);
        board.set_piece(Pos::new(1, 4), Piece::Empty);
        board.set_piece(Pos::new(1, 5), Piece::Empty);

        board.set_piece(Pos::new(2, 0), Piece::Empty);
        board.set_piece(Pos::new(2, 1), Piece::Empty);
        board.set_piece(Pos::new(2, 2), Piece::Empty);
        board.set_piece(Pos::new(2, 3), piece1);
        board.set_piece(Pos::new(2, 4), Piece::Empty);
        board.set_piece(Pos::new(2, 5), piece1);

        board.set_piece(Pos::new(3, 0), Piece::Empty);
        board.set_piece(Pos::new(3, 1), Piece::Empty);
        board.set_piece(Pos::new(3, 2), Piece::Empty);
        board.set_piece(Pos::new(3, 3), Piece::Empty);
        board.set_piece(Pos::new(3, 4), piece1);
        board.set_piece(Pos::new(3, 5), Piece::Empty);

        let expected_moves = vec![
            (Pos::new(1, 2), Pos::new(1, 1)),
            (Pos::new(2, 3), Pos::new(2, 0)),
            (Pos::new(2, 5), Pos::new(2, 1)),
            (Pos::new(3, 4), Pos::new(3, 0))
        ];
        assert_eq!(expected_moves, board.trickle());
    }

    #[test]
    fn trickle_no_diagonals_fills_prev_piece_space_sets_board() {
        let type1 = 'f';
        let piece1 = Piece::Regular(type1, ALL_DIRECTIONS);

        let mut board = Board::new(BoardState::new(16, 16), 
                                   Vec::new(), Vec::new());

        board.set_piece(Pos::new(0, 0), piece1);
        board.set_piece(Pos::new(0, 1), Piece::Empty);
        board.set_piece(Pos::new(0, 2), Piece::Empty);
        board.set_piece(Pos::new(0, 3), Piece::Empty);
        board.set_piece(Pos::new(0, 4), Piece::Empty);
        board.set_piece(Pos::new(0, 5), Piece::Empty);

        board.set_piece(Pos::new(1, 0), piece1);
        board.set_piece(Pos::new(1, 1), Piece::Empty);
        board.set_piece(Pos::new(1, 2), piece1);
        board.set_piece(Pos::new(1, 3), Piece::Empty);
        board.set_piece(Pos::new(1, 4), Piece::Empty);
        board.set_piece(Pos::new(1, 5), piece1);

        board.set_piece(Pos::new(2, 0), Piece::Empty);
        board.set_piece(Pos::new(2, 1), Piece::Empty);
        board.set_piece(Pos::new(2, 2), Piece::Empty);
        board.set_piece(Pos::new(2, 3), piece1);
        board.set_piece(Pos::new(2, 4), Piece::Empty);
        board.set_piece(Pos::new(2, 5), piece1);

        board.set_piece(Pos::new(3, 0), Piece::Empty);
        board.set_piece(Pos::new(3, 1), Piece::Empty);
        board.set_piece(Pos::new(3, 2), Piece::Empty);
        board.set_piece(Pos::new(3, 3), Piece::Empty);
        board.set_piece(Pos::new(3, 4), piece1);
        board.set_piece(Pos::new(3, 5), Piece::Empty);

        board.trickle();

        assert_eq!(piece1, board.piece(Pos::new(0, 0)));
        assert_eq!(piece1, board.piece(Pos::new(0, 1)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(0, 2)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(0, 3)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(0, 4)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(0, 5)));

        assert_eq!(piece1, board.piece(Pos::new(1, 0)));
        assert_eq!(piece1, board.piece(Pos::new(1, 1)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(1, 2)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(1, 3)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(1, 4)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(1, 5)));

        assert_eq!(piece1, board.piece(Pos::new(2, 0)));
        assert_eq!(piece1, board.piece(Pos::new(2, 1)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(2, 2)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(2, 3)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(2, 4)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(2, 5)));

        assert_eq!(piece1, board.piece(Pos::new(3, 0)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(3, 1)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(3, 2)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(3, 3)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(3, 4)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(3, 5)));
    }

    #[test]
    fn trickle_with_diagonals_fills_prev_piece_space_generates_moves() {
        let type1 = 'f';
        let piece1 = Piece::Regular(type1, ALL_DIRECTIONS);

        let mut board = Board::new(BoardState::new(16, 16), 
                                   Vec::new(), Vec::new());

        board.set_piece(Pos::new(0, 0), piece1);
        board.set_piece(Pos::new(0, 1), Piece::Empty);
        board.set_piece(Pos::new(0, 2), Piece::Empty);
        board.set_piece(Pos::new(0, 3), Piece::Empty);
        board.set_piece(Pos::new(0, 4), Piece::Empty);
        board.set_piece(Pos::new(0, 5), Piece::Empty);

        board.set_piece(Pos::new(1, 0), piece1);
        board.set_piece(Pos::new(1, 1), Piece::Empty);
        board.set_piece(Pos::new(1, 2), piece1);
        board.set_piece(Pos::new(1, 3), Piece::Empty);
        board.set_piece(Pos::new(1, 4), Piece::Empty);
        board.set_piece(Pos::new(1, 5), piece1);

        board.set_piece(Pos::new(2, 0), Piece::Empty);
        board.set_piece(Pos::new(2, 1), Piece::Empty);
        board.set_piece(Pos::new(2, 2), Piece::Empty);
        board.set_piece(Pos::new(2, 3), piece1);
        board.set_piece(Pos::new(2, 4), Piece::Empty);
        board.set_piece(Pos::new(2, 5), piece1);

        board.set_piece(Pos::new(3, 0), Piece::Empty);
        board.set_piece(Pos::new(3, 1), Piece::Empty);
        board.set_piece(Pos::new(3, 2), Piece::Empty);
        board.set_piece(Pos::new(3, 3), Piece::Empty);
        board.set_piece(Pos::new(3, 4), piece1);
        board.set_piece(Pos::new(3, 5), Piece::Empty);

        let expected_moves = vec![
            (Pos::new(1, 2), Pos::new(1, 1)),
            (Pos::new(1, 5), Pos::new(1, 2)),
            (Pos::new(2, 3), Pos::new(2, 0)),
            (Pos::new(2, 5), Pos::new(2, 1)),
            (Pos::new(3, 4), Pos::new(3, 0)),
            (Pos::new(1, 2), Pos::new(0, 1))
        ];
        assert_eq!(expected_moves, board.trickle());
    }

    #[test]
    fn trickle_with_diagonals_left_sets_board() {
        let type1 = 'f';
        let piece1 = Piece::Regular(type1, ALL_DIRECTIONS);

        let mut board = Board::new(BoardState::new(16, 16), 
                                   Vec::new(), Vec::new());

        board.set_piece(Pos::new(0, 0), piece1);
        board.set_piece(Pos::new(0, 1), Piece::Empty);
        board.set_piece(Pos::new(0, 2), Piece::Empty);
        board.set_piece(Pos::new(0, 3), Piece::Empty);
        board.set_piece(Pos::new(0, 4), Piece::Empty);
        board.set_piece(Pos::new(0, 5), Piece::Empty);

        board.set_piece(Pos::new(1, 0), piece1);
        board.set_piece(Pos::new(1, 1), Piece::Empty);
        board.set_piece(Pos::new(1, 2), piece1);
        board.set_piece(Pos::new(1, 3), Piece::Empty);
        board.set_piece(Pos::new(1, 4), Piece::Empty);
        board.set_piece(Pos::new(1, 5), piece1);

        board.set_piece(Pos::new(2, 0), Piece::Empty);
        board.set_piece(Pos::new(2, 1), Piece::Empty);
        board.set_piece(Pos::new(2, 2), Piece::Empty);
        board.set_piece(Pos::new(2, 3), piece1);
        board.set_piece(Pos::new(2, 4), Piece::Empty);
        board.set_piece(Pos::new(2, 5), piece1);

        board.set_piece(Pos::new(3, 0), Piece::Empty);
        board.set_piece(Pos::new(3, 1), Piece::Empty);
        board.set_piece(Pos::new(3, 2), Piece::Empty);
        board.set_piece(Pos::new(3, 3), Piece::Empty);
        board.set_piece(Pos::new(3, 4), piece1);
        board.set_piece(Pos::new(3, 5), Piece::Empty);

        board.trickle();

        assert_eq!(piece1, board.piece(Pos::new(0, 0)));
        assert_eq!(piece1, board.piece(Pos::new(0, 1)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(0, 2)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(0, 3)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(0, 4)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(0, 5)));

        assert_eq!(piece1, board.piece(Pos::new(1, 0)));
        assert_eq!(piece1, board.piece(Pos::new(1, 1)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(1, 2)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(1, 3)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(1, 4)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(1, 5)));

        assert_eq!(piece1, board.piece(Pos::new(2, 0)));
        assert_eq!(piece1, board.piece(Pos::new(2, 1)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(2, 2)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(2, 3)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(2, 4)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(2, 5)));

        assert_eq!(piece1, board.piece(Pos::new(3, 0)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(3, 1)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(3, 2)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(3, 3)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(3, 4)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(3, 5)));
    }

    #[test]
    fn trickle_with_diagonals_left_generates_moves() {
        let type1 = 'f';
        let piece1 = Piece::Regular(type1, ALL_DIRECTIONS);

        let mut board = Board::new(BoardState::new(16, 16), 
                                   Vec::new(), Vec::new());

        board.set_piece(Pos::new(0, 0), piece1);
        board.set_piece(Pos::new(0, 1), Piece::Empty);
        board.set_piece(Pos::new(0, 2), Piece::Empty);
        board.set_piece(Pos::new(0, 3), Piece::Empty);
        board.set_piece(Pos::new(0, 4), Piece::Empty);
        board.set_piece(Pos::new(0, 5), Piece::Empty);

        board.set_piece(Pos::new(1, 0), piece1);
        board.set_piece(Pos::new(1, 1), piece1);
        board.set_piece(Pos::new(1, 2), Piece::Empty);
        board.set_piece(Pos::new(1, 3), Piece::Empty);
        board.set_piece(Pos::new(1, 4), Piece::Empty);
        board.set_piece(Pos::new(1, 5), piece1);

        board.set_piece(Pos::new(2, 0), Piece::Empty);
        board.set_piece(Pos::new(2, 1), Piece::Empty);
        board.set_piece(Pos::new(2, 2), Piece::Empty);
        board.set_piece(Pos::new(2, 3), piece1);
        board.set_piece(Pos::new(2, 4), Piece::Empty);
        board.set_piece(Pos::new(2, 5), piece1);

        board.set_piece(Pos::new(3, 0), Piece::Empty);
        board.set_piece(Pos::new(3, 1), Piece::Empty);
        board.set_piece(Pos::new(3, 2), Piece::Empty);
        board.set_piece(Pos::new(3, 3), Piece::Empty);
        board.set_piece(Pos::new(3, 4), piece1);
        board.set_piece(Pos::new(3, 5), Piece::Empty);

        let expected_moves = vec![
            (Pos::new(1, 5), Pos::new(1, 2)),
            (Pos::new(2, 3), Pos::new(2, 0)),
            (Pos::new(2, 5), Pos::new(2, 1)),
            (Pos::new(3, 4), Pos::new(3, 0)),
            (Pos::new(1, 2), Pos::new(0, 1))
        ];
        assert_eq!(expected_moves, board.trickle());
    }

    #[test]
    fn trickle_with_diagonals_right_sets_board() {
        let type1 = 'f';
        let piece1 = Piece::Regular(type1, ALL_DIRECTIONS);

        let mut board = Board::new(BoardState::new(16, 16), 
                                   Vec::new(), Vec::new());

        board.set_piece(Pos::new(0, 0), piece1);
        board.set_piece(Pos::new(0, 1), Piece::Empty);
        board.set_piece(Pos::new(0, 2), Piece::Empty);
        board.set_piece(Pos::new(0, 3), Piece::Empty);
        board.set_piece(Pos::new(0, 4), Piece::Empty);
        board.set_piece(Pos::new(0, 5), Piece::Empty);

        board.set_piece(Pos::new(1, 0), piece1);
        board.set_piece(Pos::new(1, 1), Piece::Empty);
        board.set_piece(Pos::new(1, 2), piece1);
        board.set_piece(Pos::new(1, 3), Piece::Empty);
        board.set_piece(Pos::new(1, 4), Piece::Empty);
        board.set_piece(Pos::new(1, 5), Piece::Empty);

        board.set_piece(Pos::new(2, 0), Piece::Empty);
        board.set_piece(Pos::new(2, 1), Piece::Empty);
        board.set_piece(Pos::new(2, 2), Piece::Empty);
        board.set_piece(Pos::new(2, 3), piece1);
        board.set_piece(Pos::new(2, 4), piece1);
        board.set_piece(Pos::new(2, 5), piece1);

        board.set_piece(Pos::new(3, 0), Piece::Empty);
        board.set_piece(Pos::new(3, 1), Piece::Empty);
        board.set_piece(Pos::new(3, 2), Piece::Empty);
        board.set_piece(Pos::new(3, 3), Piece::Empty);
        board.set_piece(Pos::new(3, 4), piece1);
        board.set_piece(Pos::new(3, 5), Piece::Empty);

        board.trickle();

        assert_eq!(piece1, board.piece(Pos::new(0, 0)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(0, 1)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(0, 2)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(0, 3)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(0, 4)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(0, 5)));

        assert_eq!(piece1, board.piece(Pos::new(1, 0)));
        assert_eq!(piece1, board.piece(Pos::new(1, 1)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(1, 2)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(1, 3)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(1, 4)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(1, 5)));

        assert_eq!(piece1, board.piece(Pos::new(2, 0)));
        assert_eq!(piece1, board.piece(Pos::new(2, 1)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(2, 2)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(2, 3)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(2, 4)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(2, 5)));

        assert_eq!(piece1, board.piece(Pos::new(3, 0)));
        assert_eq!(piece1, board.piece(Pos::new(3, 1)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(3, 2)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(3, 3)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(3, 4)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(3, 5)));
    }

    #[test]
    fn trickle_with_diagonals_right_generates_moves() {
        let type1 = 'f';
        let piece1 = Piece::Regular(type1, ALL_DIRECTIONS);

        let mut board = Board::new(BoardState::new(16, 16), 
                                   Vec::new(), Vec::new());

        board.set_piece(Pos::new(0, 0), piece1);
        board.set_piece(Pos::new(0, 1), Piece::Empty);
        board.set_piece(Pos::new(0, 2), Piece::Empty);
        board.set_piece(Pos::new(0, 3), Piece::Empty);
        board.set_piece(Pos::new(0, 4), Piece::Empty);
        board.set_piece(Pos::new(0, 5), Piece::Empty);

        board.set_piece(Pos::new(1, 0), piece1);
        board.set_piece(Pos::new(1, 1), Piece::Empty);
        board.set_piece(Pos::new(1, 2), piece1);
        board.set_piece(Pos::new(1, 3), Piece::Empty);
        board.set_piece(Pos::new(1, 4), Piece::Empty);
        board.set_piece(Pos::new(1, 5), Piece::Empty);

        board.set_piece(Pos::new(2, 0), Piece::Empty);
        board.set_piece(Pos::new(2, 1), Piece::Empty);
        board.set_piece(Pos::new(2, 2), Piece::Empty);
        board.set_piece(Pos::new(2, 3), piece1);
        board.set_piece(Pos::new(2, 4), piece1);
        board.set_piece(Pos::new(2, 5), piece1);

        board.set_piece(Pos::new(3, 0), Piece::Empty);
        board.set_piece(Pos::new(3, 1), Piece::Empty);
        board.set_piece(Pos::new(3, 2), Piece::Empty);
        board.set_piece(Pos::new(3, 3), Piece::Empty);
        board.set_piece(Pos::new(3, 4), piece1);
        board.set_piece(Pos::new(3, 5), Piece::Empty);

        let expected_moves = vec![
            (Pos::new(1, 2), Pos::new(1, 1)),
            (Pos::new(2, 3), Pos::new(2, 0)),
            (Pos::new(2, 4), Pos::new(2, 1)),
            (Pos::new(2, 5), Pos::new(2, 2)),
            (Pos::new(3, 4), Pos::new(3, 0)),
            (Pos::new(2, 2), Pos::new(3, 1))
        ];
        assert_eq!(expected_moves, board.trickle());
    }

    #[test]
    fn trickle_with_diagonals_ambiguous_sets_board_left_preferred() {
        let type1 = 'f';
        let piece1 = Piece::Regular(type1, ALL_DIRECTIONS);

        let mut board = Board::new(BoardState::new(16, 16), 
                                   Vec::new(), Vec::new());

        board.set_piece(Pos::new(0, 0), piece1);
        board.set_piece(Pos::new(0, 1), Piece::Empty);
        board.set_piece(Pos::new(0, 2), Piece::Empty);
        board.set_piece(Pos::new(0, 3), Piece::Empty);
        board.set_piece(Pos::new(0, 4), Piece::Empty);
        board.set_piece(Pos::new(0, 5), Piece::Empty);

        board.set_piece(Pos::new(1, 0), piece1);
        board.set_piece(Pos::new(1, 1), Piece::Empty);
        board.set_piece(Pos::new(1, 2), Piece::Empty);
        board.set_piece(Pos::new(1, 3), Piece::Empty);
        board.set_piece(Pos::new(1, 4), Piece::Empty);
        board.set_piece(Pos::new(1, 5), Piece::Empty);

        board.set_piece(Pos::new(2, 0), Piece::Empty);
        board.set_piece(Pos::new(2, 1), Piece::Empty);
        board.set_piece(Pos::new(2, 2), Piece::Empty);
        board.set_piece(Pos::new(2, 3), piece1);
        board.set_piece(Pos::new(2, 4), piece1);
        board.set_piece(Pos::new(2, 5), piece1);

        board.set_piece(Pos::new(3, 0), Piece::Empty);
        board.set_piece(Pos::new(3, 1), Piece::Empty);
        board.set_piece(Pos::new(3, 2), Piece::Empty);
        board.set_piece(Pos::new(3, 3), Piece::Empty);
        board.set_piece(Pos::new(3, 4), piece1);
        board.set_piece(Pos::new(3, 5), Piece::Empty);

        board.trickle();

        assert_eq!(piece1, board.piece(Pos::new(0, 0)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(0, 1)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(0, 2)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(0, 3)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(0, 4)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(0, 5)));

        assert_eq!(piece1, board.piece(Pos::new(1, 0)));
        assert_eq!(piece1, board.piece(Pos::new(1, 1)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(1, 2)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(1, 3)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(1, 4)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(1, 5)));

        assert_eq!(piece1, board.piece(Pos::new(2, 0)));
        assert_eq!(piece1, board.piece(Pos::new(2, 1)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(2, 2)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(2, 3)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(2, 4)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(2, 5)));

        assert_eq!(piece1, board.piece(Pos::new(3, 0)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(3, 1)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(3, 2)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(3, 3)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(3, 4)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(3, 5)));
    }

    #[test]
    fn trickle_with_diagonals_ambiguous_generates_moves_left_preferred() {
        let type1 = 'f';
        let piece1 = Piece::Regular(type1, ALL_DIRECTIONS);

        let mut board = Board::new(BoardState::new(16, 16), 
                                   Vec::new(), Vec::new());

        board.set_piece(Pos::new(0, 0), piece1);
        board.set_piece(Pos::new(0, 1), Piece::Empty);
        board.set_piece(Pos::new(0, 2), Piece::Empty);
        board.set_piece(Pos::new(0, 3), Piece::Empty);
        board.set_piece(Pos::new(0, 4), Piece::Empty);
        board.set_piece(Pos::new(0, 5), Piece::Empty);

        board.set_piece(Pos::new(1, 0), piece1);
        board.set_piece(Pos::new(1, 1), Piece::Empty);
        board.set_piece(Pos::new(1, 2), Piece::Empty);
        board.set_piece(Pos::new(1, 3), Piece::Empty);
        board.set_piece(Pos::new(1, 4), Piece::Empty);
        board.set_piece(Pos::new(1, 5), Piece::Empty);

        board.set_piece(Pos::new(2, 0), Piece::Empty);
        board.set_piece(Pos::new(2, 1), Piece::Empty);
        board.set_piece(Pos::new(2, 2), Piece::Empty);
        board.set_piece(Pos::new(2, 3), piece1);
        board.set_piece(Pos::new(2, 4), piece1);
        board.set_piece(Pos::new(2, 5), piece1);

        board.set_piece(Pos::new(3, 0), Piece::Empty);
        board.set_piece(Pos::new(3, 1), Piece::Empty);
        board.set_piece(Pos::new(3, 2), Piece::Empty);
        board.set_piece(Pos::new(3, 3), Piece::Empty);
        board.set_piece(Pos::new(3, 4), piece1);
        board.set_piece(Pos::new(3, 5), Piece::Empty);

        let expected_moves = vec![
            (Pos::new(2, 3), Pos::new(2, 0)),
            (Pos::new(2, 4), Pos::new(2, 1)),
            (Pos::new(2, 5), Pos::new(2, 2)),
            (Pos::new(3, 4), Pos::new(3, 0)),
            (Pos::new(2, 2), Pos::new(1, 1))
        ];
        assert_eq!(expected_moves, board.trickle());
    }

    #[test]
    fn trickle_with_diagonals_tall_tower_sets_board() {
        let type1 = 'f';
        let piece1 = Piece::Regular(type1, ALL_DIRECTIONS);

        let mut board = Board::new(BoardState::new(16, 16), 
                                   Vec::new(), Vec::new());

        board.set_piece(Pos::new(0, 0), piece1);
        board.set_piece(Pos::new(0, 1), Piece::Empty);
        board.set_piece(Pos::new(0, 2), Piece::Empty);
        board.set_piece(Pos::new(0, 3), Piece::Empty);
        board.set_piece(Pos::new(0, 4), Piece::Empty);
        board.set_piece(Pos::new(0, 5), Piece::Empty);

        board.set_piece(Pos::new(1, 0), piece1);
        board.set_piece(Pos::new(1, 1), Piece::Empty);
        board.set_piece(Pos::new(1, 2), Piece::Empty);
        board.set_piece(Pos::new(1, 3), Piece::Empty);
        board.set_piece(Pos::new(1, 4), Piece::Empty);
        board.set_piece(Pos::new(1, 5), Piece::Empty);

        board.set_piece(Pos::new(2, 0), Piece::Empty);
        board.set_piece(Pos::new(2, 1), Piece::Empty);
        board.set_piece(Pos::new(2, 2), piece1);
        board.set_piece(Pos::new(2, 3), piece1);
        board.set_piece(Pos::new(2, 4), piece1);
        board.set_piece(Pos::new(2, 5), piece1);

        board.set_piece(Pos::new(3, 0), Piece::Empty);
        board.set_piece(Pos::new(3, 1), Piece::Empty);
        board.set_piece(Pos::new(3, 2), Piece::Empty);
        board.set_piece(Pos::new(3, 3), Piece::Empty);
        board.set_piece(Pos::new(3, 4), piece1);
        board.set_piece(Pos::new(3, 5), Piece::Empty);

        board.trickle();

        assert_eq!(piece1, board.piece(Pos::new(0, 0)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(0, 1)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(0, 2)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(0, 3)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(0, 4)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(0, 5)));

        assert_eq!(piece1, board.piece(Pos::new(1, 0)));
        assert_eq!(piece1, board.piece(Pos::new(1, 1)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(1, 2)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(1, 3)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(1, 4)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(1, 5)));

        assert_eq!(piece1, board.piece(Pos::new(2, 0)));
        assert_eq!(piece1, board.piece(Pos::new(2, 1)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(2, 2)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(2, 3)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(2, 4)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(2, 5)));

        assert_eq!(piece1, board.piece(Pos::new(3, 0)));
        assert_eq!(piece1, board.piece(Pos::new(3, 1)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(3, 2)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(3, 3)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(3, 4)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(3, 5)));
    }

    #[test]
    fn trickle_with_diagonals_tall_tower_generates_moves() {
        let type1 = 'f';
        let piece1 = Piece::Regular(type1, ALL_DIRECTIONS);

        let mut board = Board::new(BoardState::new(16, 16), 
                                   Vec::new(), Vec::new());

        board.set_piece(Pos::new(0, 0), piece1);
        board.set_piece(Pos::new(0, 1), Piece::Empty);
        board.set_piece(Pos::new(0, 2), Piece::Empty);
        board.set_piece(Pos::new(0, 3), Piece::Empty);
        board.set_piece(Pos::new(0, 4), Piece::Empty);
        board.set_piece(Pos::new(0, 5), Piece::Empty);

        board.set_piece(Pos::new(1, 0), piece1);
        board.set_piece(Pos::new(1, 1), Piece::Empty);
        board.set_piece(Pos::new(1, 2), Piece::Empty);
        board.set_piece(Pos::new(1, 3), Piece::Empty);
        board.set_piece(Pos::new(1, 4), Piece::Empty);
        board.set_piece(Pos::new(1, 5), Piece::Empty);

        board.set_piece(Pos::new(2, 0), Piece::Empty);
        board.set_piece(Pos::new(2, 1), Piece::Empty);
        board.set_piece(Pos::new(2, 2), piece1);
        board.set_piece(Pos::new(2, 3), piece1);
        board.set_piece(Pos::new(2, 4), piece1);
        board.set_piece(Pos::new(2, 5), piece1);

        board.set_piece(Pos::new(3, 0), Piece::Empty);
        board.set_piece(Pos::new(3, 1), Piece::Empty);
        board.set_piece(Pos::new(3, 2), Piece::Empty);
        board.set_piece(Pos::new(3, 3), Piece::Empty);
        board.set_piece(Pos::new(3, 4), piece1);
        board.set_piece(Pos::new(3, 5), Piece::Empty);

        let expected_moves = vec![
            (Pos::new(2, 2), Pos::new(2, 0)),
            (Pos::new(2, 3), Pos::new(2, 1)),
            (Pos::new(2, 4), Pos::new(2, 2)),
            (Pos::new(2, 5), Pos::new(2, 3)),
            (Pos::new(3, 4), Pos::new(3, 0)),
            (Pos::new(2, 2), Pos::new(1, 1)),
            (Pos::new(2, 3), Pos::new(2, 2)),
            (Pos::new(2, 2), Pos::new(3, 1))
        ];
        assert_eq!(expected_moves, board.trickle());
    }

    #[test]
    fn trickle_with_diagonals_blocking_wall_sets_board() {
        let type1 = 'f';
        let piece1 = Piece::Regular(type1, ALL_DIRECTIONS);

        let mut board = Board::new(BoardState::new(16, 16), 
                                   Vec::new(), Vec::new());

        board.set_piece(Pos::new(0, 0), Piece::Empty);
        board.set_piece(Pos::new(0, 1), Piece::Empty);
        board.set_piece(Pos::new(0, 2), Piece::Empty);
        board.set_piece(Pos::new(0, 3), Piece::Wall);
        board.set_piece(Pos::new(0, 4), Piece::Empty);
        board.set_piece(Pos::new(0, 5), piece1);

        board.set_piece(Pos::new(1, 0), Piece::Empty);
        board.set_piece(Pos::new(1, 1), Piece::Empty);
        board.set_piece(Pos::new(1, 2), piece1);
        board.set_piece(Pos::new(1, 3), Piece::Wall);
        board.set_piece(Pos::new(1, 4), Piece::Empty);
        board.set_piece(Pos::new(1, 5), piece1);

        board.set_piece(Pos::new(2, 0), Piece::Empty);
        board.set_piece(Pos::new(2, 1), Piece::Empty);
        board.set_piece(Pos::new(2, 2), Piece::Empty);
        board.set_piece(Pos::new(2, 3), Piece::Wall);
        board.set_piece(Pos::new(2, 4), piece1);
        board.set_piece(Pos::new(2, 5), piece1);

        board.set_piece(Pos::new(3, 0), Piece::Empty);
        board.set_piece(Pos::new(3, 1), piece1);
        board.set_piece(Pos::new(3, 2), Piece::Empty);
        board.set_piece(Pos::new(3, 3), Piece::Wall);
        board.set_piece(Pos::new(3, 4), Piece::Empty);
        board.set_piece(Pos::new(3, 5), Piece::Empty);

        board.trickle();

        assert_eq!(Piece::Empty, board.piece(Pos::new(0, 0)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(0, 1)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(0, 2)));
        assert_eq!(Piece::Wall, board.piece(Pos::new(0, 3)));
        assert_eq!(piece1, board.piece(Pos::new(0, 4)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(0, 5)));

        assert_eq!(piece1, board.piece(Pos::new(1, 0)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(1, 1)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(1, 2)));
        assert_eq!(Piece::Wall, board.piece(Pos::new(1, 3)));
        assert_eq!(piece1, board.piece(Pos::new(1, 4)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(1, 5)));

        assert_eq!(Piece::Empty, board.piece(Pos::new(2, 0)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(2, 1)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(2, 2)));
        assert_eq!(Piece::Wall, board.piece(Pos::new(2, 3)));
        assert_eq!(piece1, board.piece(Pos::new(2, 4)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(2, 5)));

        assert_eq!(piece1, board.piece(Pos::new(3, 0)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(3, 1)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(3, 2)));
        assert_eq!(Piece::Wall, board.piece(Pos::new(3, 3)));
        assert_eq!(piece1, board.piece(Pos::new(3, 4)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(3, 5)));
    }

    #[test]
    fn trickle_with_diagonals_blocking_wall_generates_moves() {
        let type1 = 'f';
        let piece1 = Piece::Regular(type1, ALL_DIRECTIONS);

        let mut board = Board::new(BoardState::new(16, 16), 
                                   Vec::new(), Vec::new());

        board.set_piece(Pos::new(0, 0), Piece::Empty);
        board.set_piece(Pos::new(0, 1), Piece::Empty);
        board.set_piece(Pos::new(0, 2), Piece::Empty);
        board.set_piece(Pos::new(0, 3), Piece::Wall);
        board.set_piece(Pos::new(0, 4), Piece::Empty);
        board.set_piece(Pos::new(0, 5), piece1);

        board.set_piece(Pos::new(1, 0), Piece::Empty);
        board.set_piece(Pos::new(1, 1), Piece::Empty);
        board.set_piece(Pos::new(1, 2), piece1);
        board.set_piece(Pos::new(1, 3), Piece::Wall);
        board.set_piece(Pos::new(1, 4), Piece::Empty);
        board.set_piece(Pos::new(1, 5), piece1);

        board.set_piece(Pos::new(2, 0), Piece::Empty);
        board.set_piece(Pos::new(2, 1), Piece::Empty);
        board.set_piece(Pos::new(2, 2), Piece::Empty);
        board.set_piece(Pos::new(2, 3), Piece::Wall);
        board.set_piece(Pos::new(2, 4), piece1);
        board.set_piece(Pos::new(2, 5), piece1);

        board.set_piece(Pos::new(3, 0), Piece::Empty);
        board.set_piece(Pos::new(3, 1), piece1);
        board.set_piece(Pos::new(3, 2), Piece::Empty);
        board.set_piece(Pos::new(3, 3), Piece::Wall);
        board.set_piece(Pos::new(3, 4), Piece::Empty);
        board.set_piece(Pos::new(3, 5), Piece::Empty);

        let expected_moves = vec![
            (Pos::new(0, 5), Pos::new(0, 4)),
            (Pos::new(1, 2), Pos::new(1, 0)),
            (Pos::new(1, 5), Pos::new(1, 4)),
            (Pos::new(3, 1), Pos::new(3, 0)),
            (Pos::new(2, 5), Pos::new(3, 4))
        ];
        assert_eq!(expected_moves, board.trickle());
    }

    #[test]
    fn trickle_with_diagonals_through_hole_sets_board() {
        let type1 = 'f';
        let piece1 = Piece::Regular(type1, ALL_DIRECTIONS);

        let mut board = Board::new(BoardState::new(16, 16), 
                                   Vec::new(), Vec::new());

        board.set_piece(Pos::new(0, 0), Piece::Empty);
        board.set_piece(Pos::new(0, 1), Piece::Empty);
        board.set_piece(Pos::new(0, 2), Piece::Empty);
        board.set_piece(Pos::new(0, 3), Piece::Empty);
        board.set_piece(Pos::new(0, 4), Piece::Wall);
        board.set_piece(Pos::new(0, 5), piece1);

        board.set_piece(Pos::new(1, 0), Piece::Empty);
        board.set_piece(Pos::new(1, 1), Piece::Empty);
        board.set_piece(Pos::new(1, 2), Piece::Empty);
        board.set_piece(Pos::new(1, 3), Piece::Empty);
        board.set_piece(Pos::new(1, 4), Piece::Wall);
        board.set_piece(Pos::new(1, 5), piece1);

        board.set_piece(Pos::new(2, 0), Piece::Empty);
        board.set_piece(Pos::new(2, 1), Piece::Empty);
        board.set_piece(Pos::new(2, 2), Piece::Empty);
        board.set_piece(Pos::new(2, 3), Piece::Empty);
        board.set_piece(Pos::new(2, 4), Piece::Empty);
        board.set_piece(Pos::new(2, 5), Piece::Empty);

        board.set_piece(Pos::new(3, 0), Piece::Empty);
        board.set_piece(Pos::new(3, 1), Piece::Empty);
        board.set_piece(Pos::new(3, 2), Piece::Empty);
        board.set_piece(Pos::new(3, 3), Piece::Empty);
        board.set_piece(Pos::new(3, 4), Piece::Wall);
        board.set_piece(Pos::new(3, 5), piece1);

        board.trickle();

        assert_eq!(Piece::Empty, board.piece(Pos::new(0, 0)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(0, 1)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(0, 2)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(0, 3)));
        assert_eq!(Piece::Wall, board.piece(Pos::new(0, 4)));
        assert_eq!(piece1, board.piece(Pos::new(0, 5)));

        assert_eq!(piece1, board.piece(Pos::new(1, 0)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(1, 1)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(1, 2)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(1, 3)));
        assert_eq!(Piece::Wall, board.piece(Pos::new(1, 4)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(1, 5)));

        assert_eq!(piece1, board.piece(Pos::new(2, 0)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(2, 1)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(2, 2)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(2, 3)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(2, 4)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(2, 5)));

        assert_eq!(Piece::Empty, board.piece(Pos::new(3, 0)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(3, 1)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(3, 2)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(3, 3)));
        assert_eq!(Piece::Wall, board.piece(Pos::new(3, 4)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(3, 5)));
    }

    #[test]
    fn trickle_with_diagonals_through_hole_generates_moves() {
        let type1 = 'f';
        let piece1 = Piece::Regular(type1, ALL_DIRECTIONS);

        let mut board = Board::new(BoardState::new(16, 16), 
                                   Vec::new(), Vec::new());

        board.set_piece(Pos::new(0, 0), Piece::Empty);
        board.set_piece(Pos::new(0, 1), Piece::Empty);
        board.set_piece(Pos::new(0, 2), Piece::Empty);
        board.set_piece(Pos::new(0, 3), Piece::Empty);
        board.set_piece(Pos::new(0, 4), Piece::Wall);
        board.set_piece(Pos::new(0, 5), piece1);

        board.set_piece(Pos::new(1, 0), Piece::Empty);
        board.set_piece(Pos::new(1, 1), Piece::Empty);
        board.set_piece(Pos::new(1, 2), Piece::Empty);
        board.set_piece(Pos::new(1, 3), Piece::Empty);
        board.set_piece(Pos::new(1, 4), Piece::Wall);
        board.set_piece(Pos::new(1, 5), piece1);

        board.set_piece(Pos::new(2, 0), Piece::Empty);
        board.set_piece(Pos::new(2, 1), Piece::Empty);
        board.set_piece(Pos::new(2, 2), Piece::Empty);
        board.set_piece(Pos::new(2, 3), Piece::Empty);
        board.set_piece(Pos::new(2, 4), Piece::Empty);
        board.set_piece(Pos::new(2, 5), Piece::Empty);

        board.set_piece(Pos::new(3, 0), Piece::Empty);
        board.set_piece(Pos::new(3, 1), Piece::Empty);
        board.set_piece(Pos::new(3, 2), Piece::Empty);
        board.set_piece(Pos::new(3, 3), Piece::Empty);
        board.set_piece(Pos::new(3, 4), Piece::Wall);
        board.set_piece(Pos::new(3, 5), piece1);

        let expected_moves = vec![
            (Pos::new(1, 5), Pos::new(2, 4)),
            (Pos::new(2, 4), Pos::new(2, 0)),
            (Pos::new(3, 5), Pos::new(2, 4)),
            (Pos::new(2, 4), Pos::new(2, 1)),
            (Pos::new(2, 1), Pos::new(1, 0))
        ];
        assert_eq!(expected_moves, board.trickle());
    }

    #[test]
    fn trickle_with_diagonals_changing_directions_sets_board() {
        let type1 = 'f';
        let piece1 = Piece::Regular(type1, ALL_DIRECTIONS);

        let mut board = Board::new(BoardState::new(16, 16), 
                                   Vec::new(), Vec::new());

        board.set_piece(Pos::new(0, 0), piece1);
        board.set_piece(Pos::new(0, 1), Piece::Empty);
        board.set_piece(Pos::new(0, 2), Piece::Empty);
        board.set_piece(Pos::new(0, 3), Piece::Empty);
        board.set_piece(Pos::new(0, 4), Piece::Wall);
        board.set_piece(Pos::new(0, 5), piece1);

        board.set_piece(Pos::new(1, 0), piece1);
        board.set_piece(Pos::new(1, 1), piece1);
        board.set_piece(Pos::new(1, 2), Piece::Empty);
        board.set_piece(Pos::new(1, 3), Piece::Empty);
        board.set_piece(Pos::new(1, 4), Piece::Wall);
        board.set_piece(Pos::new(1, 5), Piece::Empty);

        board.set_piece(Pos::new(2, 0), piece1);
        board.set_piece(Pos::new(2, 1), Piece::Empty);
        board.set_piece(Pos::new(2, 2), Piece::Empty);
        board.set_piece(Pos::new(2, 3), Piece::Empty);
        board.set_piece(Pos::new(2, 4), Piece::Empty);
        board.set_piece(Pos::new(2, 5), Piece::Empty);

        board.set_piece(Pos::new(3, 0), Piece::Empty);
        board.set_piece(Pos::new(3, 1), Piece::Empty);
        board.set_piece(Pos::new(3, 2), Piece::Empty);
        board.set_piece(Pos::new(3, 3), Piece::Empty);
        board.set_piece(Pos::new(3, 4), Piece::Wall);
        board.set_piece(Pos::new(3, 5), piece1);

        board.trickle();

        assert_eq!(piece1, board.piece(Pos::new(0, 0)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(0, 1)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(0, 2)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(0, 3)));
        assert_eq!(Piece::Wall, board.piece(Pos::new(0, 4)));
        assert_eq!(piece1, board.piece(Pos::new(0, 5)));

        assert_eq!(piece1, board.piece(Pos::new(1, 0)));
        assert_eq!(piece1, board.piece(Pos::new(1, 1)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(1, 2)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(1, 3)));
        assert_eq!(Piece::Wall, board.piece(Pos::new(1, 4)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(1, 5)));

        assert_eq!(piece1, board.piece(Pos::new(2, 0)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(2, 1)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(2, 2)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(2, 3)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(2, 4)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(2, 5)));

        assert_eq!(piece1, board.piece(Pos::new(3, 0)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(3, 1)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(3, 2)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(3, 3)));
        assert_eq!(Piece::Wall, board.piece(Pos::new(3, 4)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(3, 5)));
    }

    #[test]
    fn trickle_with_diagonals_changing_directions_generates_moves() {
        let type1 = 'f';
        let piece1 = Piece::Regular(type1, ALL_DIRECTIONS);

        let mut board = Board::new(BoardState::new(16, 16), 
                                   Vec::new(), Vec::new());

        board.set_piece(Pos::new(0, 0), piece1);
        board.set_piece(Pos::new(0, 1), Piece::Empty);
        board.set_piece(Pos::new(0, 2), Piece::Empty);
        board.set_piece(Pos::new(0, 3), Piece::Empty);
        board.set_piece(Pos::new(0, 4), Piece::Wall);
        board.set_piece(Pos::new(0, 5), piece1);

        board.set_piece(Pos::new(1, 0), piece1);
        board.set_piece(Pos::new(1, 1), piece1);
        board.set_piece(Pos::new(1, 2), Piece::Empty);
        board.set_piece(Pos::new(1, 3), Piece::Empty);
        board.set_piece(Pos::new(1, 4), Piece::Wall);
        board.set_piece(Pos::new(1, 5), Piece::Empty);

        board.set_piece(Pos::new(2, 0), piece1);
        board.set_piece(Pos::new(2, 1), Piece::Empty);
        board.set_piece(Pos::new(2, 2), Piece::Empty);
        board.set_piece(Pos::new(2, 3), Piece::Empty);
        board.set_piece(Pos::new(2, 4), Piece::Empty);
        board.set_piece(Pos::new(2, 5), Piece::Empty);

        board.set_piece(Pos::new(3, 0), Piece::Empty);
        board.set_piece(Pos::new(3, 1), Piece::Empty);
        board.set_piece(Pos::new(3, 2), Piece::Empty);
        board.set_piece(Pos::new(3, 3), Piece::Empty);
        board.set_piece(Pos::new(3, 4), Piece::Wall);
        board.set_piece(Pos::new(3, 5), piece1);

        let expected_moves = vec![
            (Pos::new(3, 5), Pos::new(2, 4)),
            (Pos::new(2, 4), Pos::new(2, 1)),
            (Pos::new(2, 1), Pos::new(3, 0))
        ];
        assert_eq!(expected_moves, board.trickle());
    }

    #[test]
    fn trickle_with_diagonals_west_wall_sets_board() {
        let type1 = 'f';
        let piece1 = Piece::Regular(type1, ALL_DIRECTIONS);

        let mut board = Board::new(BoardState::new(16, 16), 
                                   Vec::new(), Vec::new());

        board.set_piece(Pos::new(0, 0), piece1);
        board.set_piece(Pos::new(0, 1), Piece::Empty);
        board.set_piece(Pos::new(0, 2), Piece::Wall);
        board.set_piece(Pos::new(0, 3), Piece::Empty);
        board.set_piece(Pos::new(0, 4), Piece::Empty);
        board.set_piece(Pos::new(0, 5), Piece::Empty);

        board.set_piece(Pos::new(1, 0), piece1);
        board.set_piece(Pos::new(1, 1), Piece::Empty);
        board.set_piece(Pos::new(1, 2), piece1);
        board.set_piece(Pos::new(1, 3), Piece::Empty);
        board.set_piece(Pos::new(1, 4), Piece::Empty);
        board.set_piece(Pos::new(1, 5), piece1);

        board.set_piece(Pos::new(2, 0), Piece::Empty);
        board.set_piece(Pos::new(2, 1), Piece::Empty);
        board.set_piece(Pos::new(2, 2), Piece::Empty);
        board.set_piece(Pos::new(2, 3), piece1);
        board.set_piece(Pos::new(2, 4), Piece::Empty);
        board.set_piece(Pos::new(2, 5), piece1);

        board.set_piece(Pos::new(3, 0), Piece::Empty);
        board.set_piece(Pos::new(3, 1), Piece::Empty);
        board.set_piece(Pos::new(3, 2), Piece::Empty);
        board.set_piece(Pos::new(3, 3), Piece::Empty);
        board.set_piece(Pos::new(3, 4), piece1);
        board.set_piece(Pos::new(3, 5), Piece::Empty);

        board.trickle();

        assert_eq!(piece1, board.piece(Pos::new(0, 0)));
        assert_eq!(piece1, board.piece(Pos::new(0, 1)));
        assert_eq!(Piece::Wall, board.piece(Pos::new(0, 2)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(0, 3)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(0, 4)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(0, 5)));

        assert_eq!(piece1, board.piece(Pos::new(1, 0)));
        assert_eq!(piece1, board.piece(Pos::new(1, 1)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(1, 2)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(1, 3)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(1, 4)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(1, 5)));

        assert_eq!(piece1, board.piece(Pos::new(2, 0)));
        assert_eq!(piece1, board.piece(Pos::new(2, 1)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(2, 2)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(2, 3)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(2, 4)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(2, 5)));

        assert_eq!(piece1, board.piece(Pos::new(3, 0)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(3, 1)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(3, 2)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(3, 3)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(3, 4)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(3, 5)));
    }

    #[test]
    fn trickle_with_diagonals_west_wall_generates_moves() {
        let type1 = 'f';
        let piece1 = Piece::Regular(type1, ALL_DIRECTIONS);

        let mut board = Board::new(BoardState::new(16, 16), 
                                   Vec::new(), Vec::new());

        board.set_piece(Pos::new(0, 0), piece1);
        board.set_piece(Pos::new(0, 1), Piece::Empty);
        board.set_piece(Pos::new(0, 2), Piece::Wall);
        board.set_piece(Pos::new(0, 3), Piece::Empty);
        board.set_piece(Pos::new(0, 4), Piece::Empty);
        board.set_piece(Pos::new(0, 5), Piece::Empty);

        board.set_piece(Pos::new(1, 0), piece1);
        board.set_piece(Pos::new(1, 1), piece1);
        board.set_piece(Pos::new(1, 2), Piece::Empty);
        board.set_piece(Pos::new(1, 3), Piece::Empty);
        board.set_piece(Pos::new(1, 4), Piece::Empty);
        board.set_piece(Pos::new(1, 5), piece1);

        board.set_piece(Pos::new(2, 0), Piece::Empty);
        board.set_piece(Pos::new(2, 1), Piece::Empty);
        board.set_piece(Pos::new(2, 2), Piece::Empty);
        board.set_piece(Pos::new(2, 3), piece1);
        board.set_piece(Pos::new(2, 4), Piece::Empty);
        board.set_piece(Pos::new(2, 5), piece1);

        board.set_piece(Pos::new(3, 0), Piece::Empty);
        board.set_piece(Pos::new(3, 1), Piece::Empty);
        board.set_piece(Pos::new(3, 2), Piece::Empty);
        board.set_piece(Pos::new(3, 3), Piece::Empty);
        board.set_piece(Pos::new(3, 4), piece1);
        board.set_piece(Pos::new(3, 5), Piece::Empty);

        let expected_moves = vec![
            (Pos::new(1, 5), Pos::new(1, 2)),
            (Pos::new(2, 3), Pos::new(2, 0)),
            (Pos::new(2, 5), Pos::new(2, 1)),
            (Pos::new(3, 4), Pos::new(3, 0)),
            (Pos::new(1, 2), Pos::new(0, 1))
        ];
        assert_eq!(expected_moves, board.trickle());
    }

    #[test]
    fn trickle_with_diagonals_east_wall_sets_board() {
        let type1 = 'f';
        let piece1 = Piece::Regular(type1, ALL_DIRECTIONS);

        let mut board = Board::new(BoardState::new(16, 16), 
                                   Vec::new(), Vec::new());

        board.set_piece(Pos::new(0, 0), piece1);
        board.set_piece(Pos::new(0, 1), Piece::Empty);
        board.set_piece(Pos::new(0, 2), Piece::Empty);
        board.set_piece(Pos::new(0, 3), Piece::Empty);
        board.set_piece(Pos::new(0, 4), Piece::Empty);
        board.set_piece(Pos::new(0, 5), Piece::Empty);

        board.set_piece(Pos::new(1, 0), piece1);
        board.set_piece(Pos::new(1, 1), Piece::Empty);
        board.set_piece(Pos::new(1, 2), piece1);
        board.set_piece(Pos::new(1, 3), Piece::Empty);
        board.set_piece(Pos::new(1, 4), Piece::Empty);
        board.set_piece(Pos::new(1, 5), Piece::Empty);

        board.set_piece(Pos::new(2, 0), Piece::Empty);
        board.set_piece(Pos::new(2, 1), Piece::Empty);
        board.set_piece(Pos::new(2, 2), Piece::Empty);
        board.set_piece(Pos::new(2, 3), piece1);
        board.set_piece(Pos::new(2, 4), piece1);
        board.set_piece(Pos::new(2, 5), piece1);

        board.set_piece(Pos::new(3, 0), piece1);
        board.set_piece(Pos::new(3, 1), Piece::Empty);
        board.set_piece(Pos::new(3, 2), Piece::Wall);
        board.set_piece(Pos::new(3, 3), Piece::Empty);
        board.set_piece(Pos::new(3, 4), Piece::Empty);
        board.set_piece(Pos::new(3, 5), Piece::Empty);

        board.trickle();

        assert_eq!(piece1, board.piece(Pos::new(0, 0)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(0, 1)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(0, 2)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(0, 3)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(0, 4)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(0, 5)));

        assert_eq!(piece1, board.piece(Pos::new(1, 0)));
        assert_eq!(piece1, board.piece(Pos::new(1, 1)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(1, 2)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(1, 3)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(1, 4)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(1, 5)));

        assert_eq!(piece1, board.piece(Pos::new(2, 0)));
        assert_eq!(piece1, board.piece(Pos::new(2, 1)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(2, 2)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(2, 3)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(2, 4)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(2, 5)));

        assert_eq!(piece1, board.piece(Pos::new(3, 0)));
        assert_eq!(piece1, board.piece(Pos::new(3, 1)));
        assert_eq!(Piece::Wall, board.piece(Pos::new(3, 2)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(3, 3)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(3, 4)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(3, 5)));
    }

    #[test]
    fn trickle_with_diagonals_east_wall_generates_moves() {
        let type1 = 'f';
        let piece1 = Piece::Regular(type1, ALL_DIRECTIONS);

        let mut board = Board::new(BoardState::new(16, 16), 
                                   Vec::new(), Vec::new());

        board.set_piece(Pos::new(0, 0), piece1);
        board.set_piece(Pos::new(0, 1), Piece::Empty);
        board.set_piece(Pos::new(0, 2), Piece::Empty);
        board.set_piece(Pos::new(0, 3), Piece::Empty);
        board.set_piece(Pos::new(0, 4), Piece::Empty);
        board.set_piece(Pos::new(0, 5), Piece::Empty);

        board.set_piece(Pos::new(1, 0), piece1);
        board.set_piece(Pos::new(1, 1), Piece::Empty);
        board.set_piece(Pos::new(1, 2), piece1);
        board.set_piece(Pos::new(1, 3), Piece::Empty);
        board.set_piece(Pos::new(1, 4), Piece::Empty);
        board.set_piece(Pos::new(1, 5), Piece::Empty);

        board.set_piece(Pos::new(2, 0), Piece::Empty);
        board.set_piece(Pos::new(2, 1), Piece::Empty);
        board.set_piece(Pos::new(2, 2), Piece::Empty);
        board.set_piece(Pos::new(2, 3), piece1);
        board.set_piece(Pos::new(2, 4), piece1);
        board.set_piece(Pos::new(2, 5), piece1);

        board.set_piece(Pos::new(3, 0), piece1);
        board.set_piece(Pos::new(3, 1), Piece::Empty);
        board.set_piece(Pos::new(3, 2), Piece::Wall);
        board.set_piece(Pos::new(3, 3), Piece::Empty);
        board.set_piece(Pos::new(3, 4), Piece::Empty);
        board.set_piece(Pos::new(3, 5), Piece::Empty);

        let expected_moves = vec![
            (Pos::new(1, 2), Pos::new(1, 1)),
            (Pos::new(2, 3), Pos::new(2, 0)),
            (Pos::new(2, 4), Pos::new(2, 1)),
            (Pos::new(2, 5), Pos::new(2, 2)),
            (Pos::new(2, 2), Pos::new(3, 1))
        ];
        assert_eq!(expected_moves, board.trickle());
    }

    #[test]
    fn trickle_no_diagonals_unmovable_north_sets_board() {
        let type1 = 'f';
        let piece1 = Piece::Regular(type1, ALL_DIRECTIONS);
        let piece2 = Piece::Regular(type1, enum_set!(
            Direction::South | Direction::East | Direction::West
        ));

        let mut board = Board::new(BoardState::new(16, 16), 
                                   Vec::new(), Vec::new());

        board.set_piece(Pos::new(1, 0), piece1);
        board.set_piece(Pos::new(1, 1), Piece::Empty);
        board.set_piece(Pos::new(1, 2), piece2);
        board.set_piece(Pos::new(1, 3), Piece::Empty);
        board.set_piece(Pos::new(1, 4), Piece::Empty);
        board.set_piece(Pos::new(1, 5), piece1);

        board.trickle();

        assert_eq!(piece1, board.piece(Pos::new(1, 0)));
        assert_eq!(piece2, board.piece(Pos::new(1, 1)));
        assert_eq!(piece1, board.piece(Pos::new(1, 2)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(1, 3)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(1, 4)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(1, 5)));
    }

    #[test]
    fn trickle_no_diagonals_unmovable_north_generates_moves() {
        let type1 = 'f';
        let piece1 = Piece::Regular(type1, ALL_DIRECTIONS);
        let piece2 = Piece::Regular(type1, enum_set!(
            Direction::South | Direction::East | Direction::West
        ));

        let mut board = Board::new(BoardState::new(16, 16), 
                                   Vec::new(), Vec::new());

        board.set_piece(Pos::new(1, 0), piece1);
        board.set_piece(Pos::new(1, 1), Piece::Empty);
        board.set_piece(Pos::new(1, 2), piece2);
        board.set_piece(Pos::new(1, 3), Piece::Empty);
        board.set_piece(Pos::new(1, 4), Piece::Empty);
        board.set_piece(Pos::new(1, 5), piece1);

        let expected_moves = vec![
            (Pos::new(1, 2), Pos::new(1, 1)),
            (Pos::new(1, 5), Pos::new(1, 2))
        ];
        assert_eq!(expected_moves, board.trickle());
    }

    #[test]
    fn trickle_no_diagonals_unmovable_south_sets_board_for_movable() {
        let type1 = 'f';
        let piece1 = Piece::Regular(type1, ALL_DIRECTIONS);
        let piece2 = Piece::Regular(type1, enum_set!(
            Direction::North | Direction::East | Direction::West
        ));

        let mut board = Board::new(BoardState::new(16, 16), 
                                   Vec::new(), Vec::new());

        board.set_piece(Pos::new(1, 0), piece1);
        board.set_piece(Pos::new(1, 1), Piece::Empty);
        board.set_piece(Pos::new(1, 2), piece2);
        board.set_piece(Pos::new(1, 3), Piece::Empty);
        board.set_piece(Pos::new(1, 4), Piece::Empty);
        board.set_piece(Pos::new(1, 5), piece1);

        board.trickle();

        assert_eq!(piece1, board.piece(Pos::new(1, 0)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(1, 1)));
        assert_eq!(piece2, board.piece(Pos::new(1, 2)));
        assert_eq!(piece1, board.piece(Pos::new(1, 3)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(1, 4)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(1, 5)));
    }

    #[test]
    fn trickle_no_diagonals_unmovable_south_generates_moves_for_movable() {
        let type1 = 'f';
        let piece1 = Piece::Regular(type1, ALL_DIRECTIONS);
        let piece2 = Piece::Regular(type1, enum_set!(
            Direction::North | Direction::East | Direction::West
        ));

        let mut board = Board::new(BoardState::new(16, 16), 
                                   Vec::new(), Vec::new());

        board.set_piece(Pos::new(1, 0), piece1);
        board.set_piece(Pos::new(1, 1), Piece::Empty);
        board.set_piece(Pos::new(1, 2), piece2);
        board.set_piece(Pos::new(1, 3), Piece::Empty);
        board.set_piece(Pos::new(1, 4), Piece::Empty);
        board.set_piece(Pos::new(1, 5), piece1);

        let expected_moves = vec![
            (Pos::new(1, 5), Pos::new(1, 3))
        ];
        assert_eq!(expected_moves, board.trickle());
    }

    #[test]
    fn trickle_no_diagonals_unmovable_east_sets_board() {
        let type1 = 'f';
        let piece1 = Piece::Regular(type1, ALL_DIRECTIONS);
        let piece2 = Piece::Regular(type1, enum_set!(
            Direction::South | Direction::North | Direction::West
        ));

        let mut board = Board::new(BoardState::new(16, 16), 
                                   Vec::new(), Vec::new());

        board.set_piece(Pos::new(1, 0), piece1);
        board.set_piece(Pos::new(1, 1), Piece::Empty);
        board.set_piece(Pos::new(1, 2), piece2);
        board.set_piece(Pos::new(1, 3), Piece::Empty);
        board.set_piece(Pos::new(1, 4), Piece::Empty);
        board.set_piece(Pos::new(1, 5), piece1);

        board.trickle();

        assert_eq!(piece1, board.piece(Pos::new(1, 0)));
        assert_eq!(piece2, board.piece(Pos::new(1, 1)));
        assert_eq!(piece1, board.piece(Pos::new(1, 2)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(1, 3)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(1, 4)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(1, 5)));
    }

    #[test]
    fn trickle_no_diagonals_unmovable_east_generates_moves() {
        let type1 = 'f';
        let piece1 = Piece::Regular(type1, ALL_DIRECTIONS);
        let piece2 = Piece::Regular(type1, enum_set!(
            Direction::South | Direction::North | Direction::West
        ));

        let mut board = Board::new(BoardState::new(16, 16), 
                                   Vec::new(), Vec::new());

        board.set_piece(Pos::new(1, 0), piece1);
        board.set_piece(Pos::new(1, 1), Piece::Empty);
        board.set_piece(Pos::new(1, 2), piece2);
        board.set_piece(Pos::new(1, 3), Piece::Empty);
        board.set_piece(Pos::new(1, 4), Piece::Empty);
        board.set_piece(Pos::new(1, 5), piece1);

        let expected_moves = vec![
            (Pos::new(1, 2), Pos::new(1, 1)),
            (Pos::new(1, 5), Pos::new(1, 2))
        ];
        assert_eq!(expected_moves, board.trickle());
    }

    #[test]
    fn trickle_no_diagonals_unmovable_west_sets_board() {
        let type1 = 'f';
        let piece1 = Piece::Regular(type1, ALL_DIRECTIONS);
        let piece2 = Piece::Regular(type1, enum_set!(
            Direction::South | Direction::North | Direction::East
        ));

        let mut board = Board::new(BoardState::new(16, 16), 
                                   Vec::new(), Vec::new());

        board.set_piece(Pos::new(1, 0), piece1);
        board.set_piece(Pos::new(1, 1), Piece::Empty);
        board.set_piece(Pos::new(1, 2), piece2);
        board.set_piece(Pos::new(1, 3), Piece::Empty);
        board.set_piece(Pos::new(1, 4), Piece::Empty);
        board.set_piece(Pos::new(1, 5), piece1);

        board.trickle();

        assert_eq!(piece1, board.piece(Pos::new(1, 0)));
        assert_eq!(piece2, board.piece(Pos::new(1, 1)));
        assert_eq!(piece1, board.piece(Pos::new(1, 2)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(1, 3)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(1, 4)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(1, 5)));
    }

    #[test]
    fn trickle_no_diagonals_unmovable_west_generates_moves() {
        let type1 = 'f';
        let piece1 = Piece::Regular(type1, ALL_DIRECTIONS);
        let piece2 = Piece::Regular(type1, enum_set!(
            Direction::South | Direction::North | Direction::East
        ));

        let mut board = Board::new(BoardState::new(16, 16), 
                                   Vec::new(), Vec::new());

        board.set_piece(Pos::new(1, 0), piece1);
        board.set_piece(Pos::new(1, 1), Piece::Empty);
        board.set_piece(Pos::new(1, 2), piece2);
        board.set_piece(Pos::new(1, 3), Piece::Empty);
        board.set_piece(Pos::new(1, 4), Piece::Empty);
        board.set_piece(Pos::new(1, 5), piece1);

        let expected_moves = vec![
            (Pos::new(1, 2), Pos::new(1, 1)),
            (Pos::new(1, 5), Pos::new(1, 2))
        ];
        assert_eq!(expected_moves, board.trickle());
    }

    #[test]
    fn trickle_with_diagonals_left_unmovable_north_sets_board() {
        let type1 = 'f';
        let piece1 = Piece::Regular(type1, ALL_DIRECTIONS);
        let piece2 = Piece::Regular(type1, enum_set!(
            Direction::South | Direction::East | Direction::West
        ));

        let mut board = Board::new(BoardState::new(16, 16), 
                                   Vec::new(), Vec::new());

        board.set_piece(Pos::new(1, 0), piece1);
        board.set_piece(Pos::new(1, 1), Piece::Empty);
        board.set_piece(Pos::new(1, 2), piece2);
        board.set_piece(Pos::new(1, 3), Piece::Empty);
        board.set_piece(Pos::new(1, 4), Piece::Empty);
        board.set_piece(Pos::new(1, 5), piece1);

        board.set_piece(Pos::new(0, 0), Piece::Empty);
        board.set_piece(Pos::new(0, 1), Piece::Empty);
        board.set_piece(Pos::new(0, 2), Piece::Empty);
        board.set_piece(Pos::new(0, 3), Piece::Empty);
        board.set_piece(Pos::new(0, 4), Piece::Empty);
        board.set_piece(Pos::new(0, 5), Piece::Empty);

        board.trickle();

        assert_eq!(piece2, board.piece(Pos::new(0, 0)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(0, 1)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(0, 2)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(0, 3)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(0, 4)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(0, 5)));

        assert_eq!(piece1, board.piece(Pos::new(1, 0)));
        assert_eq!(piece1, board.piece(Pos::new(1, 1)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(1, 2)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(1, 3)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(1, 4)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(1, 5)));
    }

    #[test]
    fn trickle_with_diagonals_left_unmovable_north_generates_moves() {
        let type1 = 'f';
        let piece1 = Piece::Regular(type1, ALL_DIRECTIONS);
        let piece2 = Piece::Regular(type1, enum_set!(
            Direction::South | Direction::East | Direction::West
        ));

        let mut board = Board::new(BoardState::new(16, 16), 
                                   Vec::new(), Vec::new());

        board.set_piece(Pos::new(0, 0), Piece::Empty);
        board.set_piece(Pos::new(0, 1), Piece::Empty);
        board.set_piece(Pos::new(0, 2), Piece::Empty);
        board.set_piece(Pos::new(0, 3), Piece::Empty);
        board.set_piece(Pos::new(0, 4), Piece::Empty);
        board.set_piece(Pos::new(0, 5), Piece::Empty);

        board.set_piece(Pos::new(1, 0), piece1);
        board.set_piece(Pos::new(1, 1), Piece::Empty);
        board.set_piece(Pos::new(1, 2), piece2);
        board.set_piece(Pos::new(1, 3), Piece::Empty);
        board.set_piece(Pos::new(1, 4), Piece::Empty);
        board.set_piece(Pos::new(1, 5), piece1);

        let expected_moves = vec![
            (Pos::new(1, 2), Pos::new(1, 1)),
            (Pos::new(1, 5), Pos::new(1, 2)),
            (Pos::new(1, 1), Pos::new(0, 0)),
            (Pos::new(1, 2), Pos::new(1, 1))
        ];
        assert_eq!(expected_moves, board.trickle());
    }

    #[test]
    fn trickle_with_diagonals_left_unmovable_south_sets_board() {
        let type1 = 'f';
        let piece1 = Piece::Regular(type1, ALL_DIRECTIONS);
        let piece2 = Piece::Regular(type1, enum_set!(
            Direction::North | Direction::East | Direction::West
        ));

        let mut board = Board::new(BoardState::new(16, 16), 
                                   Vec::new(), Vec::new());

        board.set_piece(Pos::new(0, 0), Piece::Empty);
        board.set_piece(Pos::new(0, 1), Piece::Empty);
        board.set_piece(Pos::new(0, 2), Piece::Empty);
        board.set_piece(Pos::new(0, 3), Piece::Empty);
        board.set_piece(Pos::new(0, 4), Piece::Empty);
        board.set_piece(Pos::new(0, 5), Piece::Empty);

        board.set_piece(Pos::new(1, 0), piece1);
        board.set_piece(Pos::new(1, 1), Piece::Empty);
        board.set_piece(Pos::new(1, 2), piece2);
        board.set_piece(Pos::new(1, 3), Piece::Empty);
        board.set_piece(Pos::new(1, 4), Piece::Empty);
        board.set_piece(Pos::new(1, 5), piece1);

        board.trickle();

        assert_eq!(piece1, board.piece(Pos::new(1, 0)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(1, 1)));
        assert_eq!(piece2, board.piece(Pos::new(1, 2)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(1, 3)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(1, 4)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(1, 5)));

        assert_eq!(piece1, board.piece(Pos::new(0, 0)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(0, 1)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(0, 2)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(0, 3)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(0, 4)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(0, 5)));
    }

    #[test]
    fn trickle_with_diagonals_left_unmovable_south_generates_moves() {
        let type1 = 'f';
        let piece1 = Piece::Regular(type1, ALL_DIRECTIONS);
        let piece2 = Piece::Regular(type1, enum_set!(
            Direction::North | Direction::East | Direction::West
        ));

        let mut board = Board::new(BoardState::new(16, 16), 
                                   Vec::new(), Vec::new());

        board.set_piece(Pos::new(0, 0), Piece::Empty);
        board.set_piece(Pos::new(0, 1), Piece::Empty);
        board.set_piece(Pos::new(0, 2), Piece::Empty);
        board.set_piece(Pos::new(0, 3), Piece::Empty);
        board.set_piece(Pos::new(0, 4), Piece::Empty);
        board.set_piece(Pos::new(0, 5), Piece::Empty);

        board.set_piece(Pos::new(1, 0), piece1);
        board.set_piece(Pos::new(1, 1), Piece::Empty);
        board.set_piece(Pos::new(1, 2), piece2);
        board.set_piece(Pos::new(1, 3), Piece::Empty);
        board.set_piece(Pos::new(1, 4), Piece::Empty);
        board.set_piece(Pos::new(1, 5), piece1);

        let expected_moves = vec![
            (Pos::new(1, 5), Pos::new(1, 3)),
            (Pos::new(1, 3), Pos::new(0, 2)),
            (Pos::new(0, 2), Pos::new(0, 0))
        ];
        assert_eq!(expected_moves, board.trickle());
    }

    #[test]
    fn trickle_with_diagonals_left_unmovable_east_sets_board() {
        let type1 = 'f';
        let piece1 = Piece::Regular(type1, ALL_DIRECTIONS);
        let piece2 = Piece::Regular(type1, enum_set!(
            Direction::South | Direction::North | Direction::West
        ));

        let mut board = Board::new(BoardState::new(16, 16), 
                                   Vec::new(), Vec::new());

        board.set_piece(Pos::new(0, 0), Piece::Empty);
        board.set_piece(Pos::new(0, 1), Piece::Empty);
        board.set_piece(Pos::new(0, 2), Piece::Empty);
        board.set_piece(Pos::new(0, 3), Piece::Empty);
        board.set_piece(Pos::new(0, 4), Piece::Empty);
        board.set_piece(Pos::new(0, 5), Piece::Empty);

        board.set_piece(Pos::new(1, 0), piece1);
        board.set_piece(Pos::new(1, 1), Piece::Empty);
        board.set_piece(Pos::new(1, 2), piece2);
        board.set_piece(Pos::new(1, 3), Piece::Empty);
        board.set_piece(Pos::new(1, 4), Piece::Empty);
        board.set_piece(Pos::new(1, 5), piece1);

        board.trickle();

        assert_eq!(piece2, board.piece(Pos::new(0, 0)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(0, 1)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(0, 2)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(0, 3)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(0, 4)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(0, 5)));

        assert_eq!(piece1, board.piece(Pos::new(1, 0)));
        assert_eq!(piece1, board.piece(Pos::new(1, 1)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(1, 2)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(1, 3)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(1, 4)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(1, 5)));
    }

    #[test]
    fn trickle_with_diagonals_left_unmovable_east_generates_moves() {
        let type1 = 'f';
        let piece1 = Piece::Regular(type1, ALL_DIRECTIONS);
        let piece2 = Piece::Regular(type1, enum_set!(
            Direction::South | Direction::North | Direction::West
        ));

        let mut board = Board::new(BoardState::new(16, 16), 
                                   Vec::new(), Vec::new());
        board.set_piece(Pos::new(0, 0), Piece::Empty);
        board.set_piece(Pos::new(0, 1), Piece::Empty);
        board.set_piece(Pos::new(0, 2), Piece::Empty);
        board.set_piece(Pos::new(0, 3), Piece::Empty);
        board.set_piece(Pos::new(0, 4), Piece::Empty);
        board.set_piece(Pos::new(0, 5), Piece::Empty);

        board.set_piece(Pos::new(1, 0), piece1);
        board.set_piece(Pos::new(1, 1), Piece::Empty);
        board.set_piece(Pos::new(1, 2), piece2);
        board.set_piece(Pos::new(1, 3), Piece::Empty);
        board.set_piece(Pos::new(1, 4), Piece::Empty);
        board.set_piece(Pos::new(1, 5), piece1);

        let expected_moves = vec![
            (Pos::new(1, 2), Pos::new(1, 1)),
            (Pos::new(1, 5), Pos::new(1, 2)),
            (Pos::new(1, 1), Pos::new(0, 0)),
            (Pos::new(1, 2), Pos::new(1, 1))
        ];
        assert_eq!(expected_moves, board.trickle());
    }

    #[test]
    fn trickle_with_diagonals_left_unmovable_west_sets_board() {
        let type1 = 'f';
        let piece1 = Piece::Regular(type1, ALL_DIRECTIONS);
        let piece2 = Piece::Regular(type1, enum_set!(
            Direction::South | Direction::East | Direction::North
        ));

        let mut board = Board::new(BoardState::new(16, 16), 
                                   Vec::new(), Vec::new());

        board.set_piece(Pos::new(0, 0), Piece::Empty);
        board.set_piece(Pos::new(0, 1), Piece::Empty);
        board.set_piece(Pos::new(0, 2), Piece::Empty);
        board.set_piece(Pos::new(0, 3), Piece::Empty);
        board.set_piece(Pos::new(0, 4), Piece::Empty);
        board.set_piece(Pos::new(0, 5), Piece::Empty);

        board.set_piece(Pos::new(1, 0), piece1);
        board.set_piece(Pos::new(1, 1), Piece::Empty);
        board.set_piece(Pos::new(1, 2), piece2);
        board.set_piece(Pos::new(1, 3), Piece::Empty);
        board.set_piece(Pos::new(1, 4), Piece::Empty);
        board.set_piece(Pos::new(1, 5), piece1);

        board.trickle();

        assert_eq!(piece1, board.piece(Pos::new(0, 0)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(0, 1)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(0, 2)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(0, 3)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(0, 4)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(0, 5)));

        assert_eq!(piece1, board.piece(Pos::new(1, 0)));
        assert_eq!(piece2, board.piece(Pos::new(1, 1)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(1, 2)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(1, 3)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(1, 4)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(1, 5)));
    }

    #[test]
    fn trickle_with_diagonals_left_unmovable_west_generates_moves() {
        let type1 = 'f';
        let piece1 = Piece::Regular(type1, ALL_DIRECTIONS);
        let piece2 = Piece::Regular(type1, enum_set!(
            Direction::South | Direction::East | Direction::North
        ));

        let mut board = Board::new(BoardState::new(16, 16), 
                                   Vec::new(), Vec::new());

        board.set_piece(Pos::new(0, 0), Piece::Empty);
        board.set_piece(Pos::new(0, 1), Piece::Empty);
        board.set_piece(Pos::new(0, 2), Piece::Empty);
        board.set_piece(Pos::new(0, 3), Piece::Empty);
        board.set_piece(Pos::new(0, 4), Piece::Empty);
        board.set_piece(Pos::new(0, 5), Piece::Empty);

        board.set_piece(Pos::new(1, 0), piece1);
        board.set_piece(Pos::new(1, 1), Piece::Empty);
        board.set_piece(Pos::new(1, 2), piece2);
        board.set_piece(Pos::new(1, 3), Piece::Empty);
        board.set_piece(Pos::new(1, 4), Piece::Empty);
        board.set_piece(Pos::new(1, 5), piece1);

        let expected_moves = vec![
            (Pos::new(1, 2), Pos::new(1, 1)),
            (Pos::new(1, 5), Pos::new(1, 2)),
            (Pos::new(1, 2), Pos::new(0, 1)),
            (Pos::new(0, 1), Pos::new(0, 0))
        ];
        assert_eq!(expected_moves, board.trickle());
    }

    #[test]
    fn trickle_with_diagonals_right_unmovable_north_sets_board() {
        let type1 = 'f';
        let piece1 = Piece::Regular(type1, ALL_DIRECTIONS);
        let piece2 = Piece::Regular(type1, enum_set!(
            Direction::South | Direction::East | Direction::West
        ));

        let mut board = Board::new(BoardState::new(16, 16), 
                                   Vec::new(), Vec::new());

        board.set_piece(Pos::new(1, 0), piece1);
        board.set_piece(Pos::new(1, 1), Piece::Empty);
        board.set_piece(Pos::new(1, 2), piece2);
        board.set_piece(Pos::new(1, 3), Piece::Empty);
        board.set_piece(Pos::new(1, 4), Piece::Empty);
        board.set_piece(Pos::new(1, 5), piece1);

        board.set_piece(Pos::new(2, 0), Piece::Empty);
        board.set_piece(Pos::new(2, 1), Piece::Empty);
        board.set_piece(Pos::new(2, 2), Piece::Empty);
        board.set_piece(Pos::new(2, 3), Piece::Empty);
        board.set_piece(Pos::new(2, 4), Piece::Empty);
        board.set_piece(Pos::new(2, 5), Piece::Empty);

        board.trickle();

        assert_eq!(piece1, board.piece(Pos::new(1, 0)));
        assert_eq!(piece1, board.piece(Pos::new(1, 1)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(1, 2)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(1, 3)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(1, 4)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(1, 5)));

        assert_eq!(piece2, board.piece(Pos::new(2, 0)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(2, 1)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(2, 2)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(2, 3)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(2, 4)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(2, 5)));
    }

    #[test]
    fn trickle_with_diagonals_right_unmovable_north_generates_moves() {
        let type1 = 'f';
        let piece1 = Piece::Regular(type1, ALL_DIRECTIONS);
        let piece2 = Piece::Regular(type1, enum_set!(
            Direction::South | Direction::East | Direction::West
        ));

        let mut board = Board::new(BoardState::new(16, 16), 
                                   Vec::new(), Vec::new());

        board.set_piece(Pos::new(1, 0), piece1);
        board.set_piece(Pos::new(1, 1), Piece::Empty);
        board.set_piece(Pos::new(1, 2), piece2);
        board.set_piece(Pos::new(1, 3), Piece::Empty);
        board.set_piece(Pos::new(1, 4), Piece::Empty);
        board.set_piece(Pos::new(1, 5), piece1);

        board.set_piece(Pos::new(2, 0), Piece::Empty);
        board.set_piece(Pos::new(2, 1), Piece::Empty);
        board.set_piece(Pos::new(2, 2), Piece::Empty);
        board.set_piece(Pos::new(2, 3), Piece::Empty);
        board.set_piece(Pos::new(2, 4), Piece::Empty);
        board.set_piece(Pos::new(2, 5), Piece::Empty);

        let expected_moves = vec![
            (Pos::new(1, 2), Pos::new(1, 1)),
            (Pos::new(1, 5), Pos::new(1, 2)),
            (Pos::new(1, 1), Pos::new(2, 0)),
            (Pos::new(1, 2), Pos::new(1, 1))
        ];
        assert_eq!(expected_moves, board.trickle());
    }

    #[test]
    fn trickle_with_diagonals_right_unmovable_south_sets_board() {
        let type1 = 'f';
        let piece1 = Piece::Regular(type1, ALL_DIRECTIONS);
        let piece2 = Piece::Regular(type1, enum_set!(
            Direction::North | Direction::East | Direction::West
        ));

        let mut board = Board::new(BoardState::new(16, 16), 
                                   Vec::new(), Vec::new());

        board.set_piece(Pos::new(1, 0), piece1);
        board.set_piece(Pos::new(1, 1), Piece::Empty);
        board.set_piece(Pos::new(1, 2), piece2);
        board.set_piece(Pos::new(1, 3), Piece::Empty);
        board.set_piece(Pos::new(1, 4), Piece::Empty);
        board.set_piece(Pos::new(1, 5), piece1);

        board.set_piece(Pos::new(2, 0), Piece::Empty);
        board.set_piece(Pos::new(2, 1), Piece::Empty);
        board.set_piece(Pos::new(2, 2), Piece::Empty);
        board.set_piece(Pos::new(2, 3), Piece::Empty);
        board.set_piece(Pos::new(2, 4), Piece::Empty);
        board.set_piece(Pos::new(2, 5), Piece::Empty);

        board.trickle();

        assert_eq!(piece1, board.piece(Pos::new(1, 0)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(1, 1)));
        assert_eq!(piece2, board.piece(Pos::new(1, 2)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(1, 3)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(1, 4)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(1, 5)));

        assert_eq!(piece1, board.piece(Pos::new(2, 0)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(2, 1)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(2, 2)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(2, 3)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(2, 4)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(2, 5)));
    }

    #[test]
    fn trickle_with_diagonals_right_unmovable_south_generates_moves() {
        let type1 = 'f';
        let piece1 = Piece::Regular(type1, ALL_DIRECTIONS);
        let piece2 = Piece::Regular(type1, enum_set!(
            Direction::North | Direction::East | Direction::West
        ));

        let mut board = Board::new(BoardState::new(16, 16), 
                                   Vec::new(), Vec::new());

        board.set_piece(Pos::new(1, 0), piece1);
        board.set_piece(Pos::new(1, 1), Piece::Empty);
        board.set_piece(Pos::new(1, 2), piece2);
        board.set_piece(Pos::new(1, 3), Piece::Empty);
        board.set_piece(Pos::new(1, 4), Piece::Empty);
        board.set_piece(Pos::new(1, 5), piece1);

        board.set_piece(Pos::new(2, 0), Piece::Empty);
        board.set_piece(Pos::new(2, 1), Piece::Empty);
        board.set_piece(Pos::new(2, 2), Piece::Empty);
        board.set_piece(Pos::new(2, 3), Piece::Empty);
        board.set_piece(Pos::new(2, 4), Piece::Empty);
        board.set_piece(Pos::new(2, 5), Piece::Empty);

        let expected_moves = vec![
            (Pos::new(1, 5), Pos::new(1, 3)),
            (Pos::new(1, 3), Pos::new(2, 2)),
            (Pos::new(2, 2), Pos::new(2, 0))
        ];
        assert_eq!(expected_moves, board.trickle());
    }

    #[test]
    fn trickle_with_diagonals_right_unmovable_east_sets_board() {
        let type1 = 'f';
        let piece1 = Piece::Regular(type1, ALL_DIRECTIONS);
        let piece2 = Piece::Regular(type1, enum_set!(
            Direction::South | Direction::North | Direction::West
        ));

        let mut board = Board::new(BoardState::new(16, 16), 
                                   Vec::new(), Vec::new());

        board.set_piece(Pos::new(1, 0), piece1);
        board.set_piece(Pos::new(1, 1), Piece::Empty);
        board.set_piece(Pos::new(1, 2), piece2);
        board.set_piece(Pos::new(1, 3), Piece::Empty);
        board.set_piece(Pos::new(1, 4), Piece::Empty);
        board.set_piece(Pos::new(1, 5), piece1);

        board.set_piece(Pos::new(2, 0), Piece::Empty);
        board.set_piece(Pos::new(2, 1), Piece::Empty);
        board.set_piece(Pos::new(2, 2), Piece::Empty);
        board.set_piece(Pos::new(2, 3), Piece::Empty);
        board.set_piece(Pos::new(2, 4), Piece::Empty);
        board.set_piece(Pos::new(2, 5), Piece::Empty);

        board.trickle();

        assert_eq!(piece1, board.piece(Pos::new(1, 0)));
        assert_eq!(piece2, board.piece(Pos::new(1, 1)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(1, 2)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(1, 3)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(1, 4)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(1, 5)));

        assert_eq!(piece1, board.piece(Pos::new(2, 0)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(2, 1)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(2, 2)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(2, 3)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(2, 4)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(2, 5)));
    }

    #[test]
    fn trickle_with_diagonals_right_unmovable_east_generates_moves() {
        let type1 = 'f';
        let piece1 = Piece::Regular(type1, ALL_DIRECTIONS);
        let piece2 = Piece::Regular(type1, enum_set!(
            Direction::South | Direction::North | Direction::West
        ));

        let mut board = Board::new(BoardState::new(16, 16), 
                                   Vec::new(), Vec::new());

        board.set_piece(Pos::new(1, 0), piece1);
        board.set_piece(Pos::new(1, 1), Piece::Empty);
        board.set_piece(Pos::new(1, 2), piece2);
        board.set_piece(Pos::new(1, 3), Piece::Empty);
        board.set_piece(Pos::new(1, 4), Piece::Empty);
        board.set_piece(Pos::new(1, 5), piece1);

        board.set_piece(Pos::new(2, 0), Piece::Empty);
        board.set_piece(Pos::new(2, 1), Piece::Empty);
        board.set_piece(Pos::new(2, 2), Piece::Empty);
        board.set_piece(Pos::new(2, 3), Piece::Empty);
        board.set_piece(Pos::new(2, 4), Piece::Empty);
        board.set_piece(Pos::new(2, 5), Piece::Empty);

        let expected_moves = vec![
            (Pos::new(1, 2), Pos::new(1, 1)),
            (Pos::new(1, 5), Pos::new(1, 2)),
            (Pos::new(1, 2), Pos::new(2, 1)),
            (Pos::new(2, 1), Pos::new(2, 0))
        ];
        assert_eq!(expected_moves, board.trickle());
    }

    #[test]
    fn trickle_with_diagonals_right_unmovable_west_sets_board() {
        let type1 = 'f';
        let piece1 = Piece::Regular(type1, ALL_DIRECTIONS);
        let piece2 = Piece::Regular(type1, enum_set!(
            Direction::South | Direction::East | Direction::North
        ));

        let mut board = Board::new(BoardState::new(16, 16), 
                                   Vec::new(), Vec::new());

        board.set_piece(Pos::new(1, 0), piece1);
        board.set_piece(Pos::new(1, 1), Piece::Empty);
        board.set_piece(Pos::new(1, 2), piece2);
        board.set_piece(Pos::new(1, 3), Piece::Empty);
        board.set_piece(Pos::new(1, 4), Piece::Empty);
        board.set_piece(Pos::new(1, 5), piece1);

        board.set_piece(Pos::new(2, 0), Piece::Empty);
        board.set_piece(Pos::new(2, 1), Piece::Empty);
        board.set_piece(Pos::new(2, 2), Piece::Empty);
        board.set_piece(Pos::new(2, 3), Piece::Empty);
        board.set_piece(Pos::new(2, 4), Piece::Empty);
        board.set_piece(Pos::new(2, 5), Piece::Empty);

        board.trickle();

        assert_eq!(piece1, board.piece(Pos::new(1, 0)));
        assert_eq!(piece1, board.piece(Pos::new(1, 1)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(1, 2)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(1, 3)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(1, 4)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(1, 5)));

        assert_eq!(piece2, board.piece(Pos::new(2, 0)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(2, 1)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(2, 2)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(2, 3)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(2, 4)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(2, 5)));
    }

    #[test]
    fn trickle_with_diagonals_right_unmovable_west_generates_moves() {
        let type1 = 'f';
        let piece1 = Piece::Regular(type1, ALL_DIRECTIONS);
        let piece2 = Piece::Regular(type1, enum_set!(
            Direction::South | Direction::East | Direction::North
        ));

        let mut board = Board::new(BoardState::new(16, 16), 
                                   Vec::new(), Vec::new());

        board.set_piece(Pos::new(1, 0), piece1);
        board.set_piece(Pos::new(1, 1), Piece::Empty);
        board.set_piece(Pos::new(1, 2), piece2);
        board.set_piece(Pos::new(1, 3), Piece::Empty);
        board.set_piece(Pos::new(1, 4), Piece::Empty);
        board.set_piece(Pos::new(1, 5), piece1);

        board.set_piece(Pos::new(2, 0), Piece::Empty);
        board.set_piece(Pos::new(2, 1), Piece::Empty);
        board.set_piece(Pos::new(2, 2), Piece::Empty);
        board.set_piece(Pos::new(2, 3), Piece::Empty);
        board.set_piece(Pos::new(2, 4), Piece::Empty);
        board.set_piece(Pos::new(2, 5), Piece::Empty);

        let expected_moves = vec![
            (Pos::new(1, 2), Pos::new(1, 1)),
            (Pos::new(1, 5), Pos::new(1, 2)),
            (Pos::new(1, 1), Pos::new(2, 0)),
            (Pos::new(1, 2), Pos::new(1, 1))
        ];
        assert_eq!(expected_moves, board.trickle());
    }

    #[test]
    fn trickle_right_border_sets_board() {
        let type1 = 'f';
        let piece1 = Piece::Regular(type1, ALL_DIRECTIONS);

        let mut board = Board::new(BoardState::new(16, 16), 
                                   Vec::new(), Vec::new());

        board.set_piece(Pos::new(15, 0), piece1);
        board.set_piece(Pos::new(15, 1), Piece::Empty);
        board.set_piece(Pos::new(15, 2), piece1);
        board.set_piece(Pos::new(15, 3), Piece::Empty);
        board.set_piece(Pos::new(15, 4), Piece::Empty);
        board.set_piece(Pos::new(15, 5), piece1);

        board.trickle();

        assert_eq!(piece1, board.piece(Pos::new(15, 0)));
        assert_eq!(piece1, board.piece(Pos::new(15, 1)));
        assert_eq!(piece1, board.piece(Pos::new(15, 2)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(15, 3)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(15, 4)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(15, 5)));
    }

    #[test]
    fn trickle_right_border_generates_moves() {
        let type1 = 'f';
        let piece1 = Piece::Regular(type1, ALL_DIRECTIONS);

        let mut board = Board::new(BoardState::new(16, 16), 
                                   Vec::new(), Vec::new());

        board.set_piece(Pos::new(15, 0), piece1);
        board.set_piece(Pos::new(15, 1), Piece::Empty);
        board.set_piece(Pos::new(15, 2), piece1);
        board.set_piece(Pos::new(15, 3), Piece::Empty);
        board.set_piece(Pos::new(15, 4), Piece::Empty);
        board.set_piece(Pos::new(15, 5), piece1);

        let expected_moves = vec![
            (Pos::new(15, 2), Pos::new(15, 1)),
            (Pos::new(15, 5), Pos::new(15, 2))
        ];
        assert_eq!(expected_moves, board.trickle());
    }

    #[test]
    fn trickle_top_border_sets_board() {
        let type1 = 'f';
        let piece1 = Piece::Regular(type1, ALL_DIRECTIONS);

        let mut board = Board::new(BoardState::new(16, 16), 
                                   Vec::new(), Vec::new());

        board.set_piece(Pos::new(0, 10), piece1);
        board.set_piece(Pos::new(0, 11), Piece::Empty);
        board.set_piece(Pos::new(0, 12), piece1);
        board.set_piece(Pos::new(0, 13), Piece::Empty);
        board.set_piece(Pos::new(0, 14), Piece::Empty);
        board.set_piece(Pos::new(0, 15), piece1);

        board.trickle();

        assert_eq!(piece1, board.piece(Pos::new(0, 10)));
        assert_eq!(piece1, board.piece(Pos::new(0, 11)));
        assert_eq!(piece1, board.piece(Pos::new(0, 12)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(0, 13)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(0, 14)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(0, 15)));
    }

    #[test]
    fn trickle_top_border_generates_moves() {
        let type1 = 'f';
        let piece1 = Piece::Regular(type1, ALL_DIRECTIONS);

        let mut board = Board::new(BoardState::new(16, 16), 
                                   Vec::new(), Vec::new());

        board.set_piece(Pos::new(0, 10), piece1);
        board.set_piece(Pos::new(0, 11), Piece::Empty);
        board.set_piece(Pos::new(0, 12), piece1);
        board.set_piece(Pos::new(0, 13), Piece::Empty);
        board.set_piece(Pos::new(0, 14), Piece::Empty);
        board.set_piece(Pos::new(0, 15), piece1);

        let expected_moves = vec![
            (Pos::new(0, 12), Pos::new(0, 11)),
            (Pos::new(0, 15), Pos::new(0, 12))
        ];
        assert_eq!(expected_moves, board.trickle());
    }

    #[test]
    fn trickle_with_diagonals_left_adjacent_even_towers_sets_board() {
        let type1 = 'f';
        let piece1 = Piece::Regular(type1, ALL_DIRECTIONS);

        let mut board = Board::new(BoardState::new(16, 16), 
                                   Vec::new(), Vec::new());

        board.set_piece(Pos::new(0, 0), Piece::Empty);
        board.set_piece(Pos::new(0, 1), Piece::Empty);
        board.set_piece(Pos::new(0, 2), Piece::Empty);
        board.set_piece(Pos::new(0, 3), Piece::Empty);
        board.set_piece(Pos::new(0, 4), Piece::Empty);
        board.set_piece(Pos::new(0, 5), Piece::Empty);

        board.set_piece(Pos::new(1, 0), piece1);
        board.set_piece(Pos::new(1, 1), piece1);
        board.set_piece(Pos::new(1, 2), piece1);
        board.set_piece(Pos::new(1, 3), Piece::Empty);
        board.set_piece(Pos::new(1, 4), Piece::Empty);
        board.set_piece(Pos::new(1, 5), Piece::Empty);

        board.set_piece(Pos::new(2, 0), piece1);
        board.set_piece(Pos::new(2, 1), piece1);
        board.set_piece(Pos::new(2, 2), piece1);
        board.set_piece(Pos::new(2, 3), Piece::Empty);
        board.set_piece(Pos::new(2, 4), Piece::Empty);
        board.set_piece(Pos::new(2, 5), Piece::Empty);

        board.trickle();

        assert_eq!(piece1, board.piece(Pos::new(0, 0)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(0, 1)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(0, 2)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(0, 3)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(0, 4)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(0, 5)));

        assert_eq!(piece1, board.piece(Pos::new(1, 0)));
        assert_eq!(piece1, board.piece(Pos::new(1, 1)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(1, 2)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(1, 3)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(1, 4)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(1, 5)));

        assert_eq!(piece1, board.piece(Pos::new(2, 0)));
        assert_eq!(piece1, board.piece(Pos::new(2, 1)));
        assert_eq!(piece1, board.piece(Pos::new(2, 2)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(2, 3)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(2, 4)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(2, 5)));
    }

    #[test]
    fn trickle_with_diagonals_left_adjacent_even_towers_generates_moves() {
        let type1 = 'f';
        let piece1 = Piece::Regular(type1, ALL_DIRECTIONS);

        let mut board = Board::new(BoardState::new(16, 16), 
                                   Vec::new(), Vec::new());

        board.set_piece(Pos::new(0, 0), Piece::Empty);
        board.set_piece(Pos::new(0, 1), Piece::Empty);
        board.set_piece(Pos::new(0, 2), Piece::Empty);
        board.set_piece(Pos::new(0, 3), Piece::Empty);
        board.set_piece(Pos::new(0, 4), Piece::Empty);
        board.set_piece(Pos::new(0, 5), Piece::Empty);

        board.set_piece(Pos::new(1, 0), piece1);
        board.set_piece(Pos::new(1, 1), piece1);
        board.set_piece(Pos::new(1, 2), piece1);
        board.set_piece(Pos::new(1, 3), Piece::Empty);
        board.set_piece(Pos::new(1, 4), Piece::Empty);
        board.set_piece(Pos::new(1, 5), Piece::Empty);

        board.set_piece(Pos::new(2, 0), piece1);
        board.set_piece(Pos::new(2, 1), piece1);
        board.set_piece(Pos::new(2, 2), piece1);
        board.set_piece(Pos::new(2, 3), Piece::Empty);
        board.set_piece(Pos::new(2, 4), Piece::Empty);
        board.set_piece(Pos::new(2, 5), Piece::Empty);

        let expected_moves = vec![
            (Pos::new(1, 1), Pos::new(0, 0)),
            (Pos::new(1, 2), Pos::new(1, 1))
        ];
        assert_eq!(expected_moves, board.trickle());
    }

    #[test]
    fn trickle_with_diagonals_right_adjacent_even_towers_sets_board() {
        let type1 = 'f';
        let piece1 = Piece::Regular(type1, ALL_DIRECTIONS);

        let mut board = Board::new(BoardState::new(16, 16), 
                                   Vec::new(), Vec::new());

        board.set_piece(Pos::new(1, 0), piece1);
        board.set_piece(Pos::new(1, 1), piece1);
        board.set_piece(Pos::new(1, 2), piece1);
        board.set_piece(Pos::new(1, 3), Piece::Empty);
        board.set_piece(Pos::new(1, 4), Piece::Empty);
        board.set_piece(Pos::new(1, 5), Piece::Empty);

        board.set_piece(Pos::new(2, 0), piece1);
        board.set_piece(Pos::new(2, 1), piece1);
        board.set_piece(Pos::new(2, 2), piece1);
        board.set_piece(Pos::new(2, 3), Piece::Empty);
        board.set_piece(Pos::new(2, 4), Piece::Empty);
        board.set_piece(Pos::new(2, 5), Piece::Empty);

        board.set_piece(Pos::new(3, 0), Piece::Empty);
        board.set_piece(Pos::new(3, 1), Piece::Empty);
        board.set_piece(Pos::new(3, 2), Piece::Empty);
        board.set_piece(Pos::new(3, 3), Piece::Empty);
        board.set_piece(Pos::new(3, 4), Piece::Empty);
        board.set_piece(Pos::new(3, 5), Piece::Empty);

        board.trickle();

        assert_eq!(piece1, board.piece(Pos::new(1, 0)));
        assert_eq!(piece1, board.piece(Pos::new(1, 1)));
        assert_eq!(piece1, board.piece(Pos::new(1, 2)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(1, 3)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(1, 4)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(1, 5)));

        assert_eq!(piece1, board.piece(Pos::new(2, 0)));
        assert_eq!(piece1, board.piece(Pos::new(2, 1)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(2, 2)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(2, 3)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(2, 4)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(2, 5)));

        assert_eq!(piece1, board.piece(Pos::new(3, 0)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(3, 1)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(3, 2)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(3, 3)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(3, 4)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(3, 5)));
    }

    #[test]
    fn trickle_with_diagonals_right_adjacent_even_towers_generates_moves() {
        let type1 = 'f';
        let piece1 = Piece::Regular(type1, ALL_DIRECTIONS);

        let mut board = Board::new(BoardState::new(16, 16), 
                                   Vec::new(), Vec::new());

        board.set_piece(Pos::new(1, 0), piece1);
        board.set_piece(Pos::new(1, 1), piece1);
        board.set_piece(Pos::new(1, 2), piece1);
        board.set_piece(Pos::new(1, 3), Piece::Empty);
        board.set_piece(Pos::new(1, 4), Piece::Empty);
        board.set_piece(Pos::new(1, 5), Piece::Empty);

        board.set_piece(Pos::new(2, 0), piece1);
        board.set_piece(Pos::new(2, 1), piece1);
        board.set_piece(Pos::new(2, 2), piece1);
        board.set_piece(Pos::new(2, 3), Piece::Empty);
        board.set_piece(Pos::new(2, 4), Piece::Empty);
        board.set_piece(Pos::new(2, 5), Piece::Empty);

        board.set_piece(Pos::new(3, 0), Piece::Empty);
        board.set_piece(Pos::new(3, 1), Piece::Empty);
        board.set_piece(Pos::new(3, 2), Piece::Empty);
        board.set_piece(Pos::new(3, 3), Piece::Empty);
        board.set_piece(Pos::new(3, 4), Piece::Empty);
        board.set_piece(Pos::new(3, 5), Piece::Empty);

        let expected_moves = vec![
            (Pos::new(2, 1), Pos::new(3, 0)),
            (Pos::new(2, 2), Pos::new(2, 1))
        ];
        assert_eq!(expected_moves, board.trickle());
    }

    #[test]
    fn trickle_with_diagonals_left_adjacent_uneven_towers_sets_board() {
        let type1 = 'f';
        let piece1 = Piece::Regular(type1, ALL_DIRECTIONS);

        let mut board = Board::new(BoardState::new(16, 16), 
                                   Vec::new(), Vec::new());

        board.set_piece(Pos::new(0, 0), Piece::Empty);
        board.set_piece(Pos::new(0, 1), Piece::Empty);
        board.set_piece(Pos::new(0, 2), Piece::Empty);
        board.set_piece(Pos::new(0, 3), Piece::Empty);
        board.set_piece(Pos::new(0, 4), Piece::Empty);
        board.set_piece(Pos::new(0, 5), Piece::Empty);

        board.set_piece(Pos::new(1, 0), piece1);
        board.set_piece(Pos::new(1, 1), piece1);
        board.set_piece(Pos::new(1, 2), Piece::Empty);
        board.set_piece(Pos::new(1, 3), Piece::Empty);
        board.set_piece(Pos::new(1, 4), Piece::Empty);
        board.set_piece(Pos::new(1, 5), Piece::Empty);

        board.set_piece(Pos::new(2, 0), piece1);
        board.set_piece(Pos::new(2, 1), piece1);
        board.set_piece(Pos::new(2, 2), piece1);
        board.set_piece(Pos::new(2, 3), piece1);
        board.set_piece(Pos::new(2, 4), Piece::Empty);
        board.set_piece(Pos::new(2, 5), Piece::Empty);

        board.trickle();

        assert_eq!(piece1, board.piece(Pos::new(0, 0)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(0, 1)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(0, 2)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(0, 3)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(0, 4)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(0, 5)));

        assert_eq!(piece1, board.piece(Pos::new(1, 0)));
        assert_eq!(piece1, board.piece(Pos::new(1, 1)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(1, 2)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(1, 3)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(1, 4)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(1, 5)));

        assert_eq!(piece1, board.piece(Pos::new(2, 0)));
        assert_eq!(piece1, board.piece(Pos::new(2, 1)));
        assert_eq!(piece1, board.piece(Pos::new(2, 2)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(2, 3)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(2, 4)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(2, 5)));
    }

    #[test]
    fn trickle_with_diagonals_left_adjacent_uneven_towers_generates_moves() {
        let type1 = 'f';
        let piece1 = Piece::Regular(type1, ALL_DIRECTIONS);

        let mut board = Board::new(BoardState::new(16, 16), 
                                   Vec::new(), Vec::new());

        board.set_piece(Pos::new(0, 0), Piece::Empty);
        board.set_piece(Pos::new(0, 1), Piece::Empty);
        board.set_piece(Pos::new(0, 2), Piece::Empty);
        board.set_piece(Pos::new(0, 3), Piece::Empty);
        board.set_piece(Pos::new(0, 4), Piece::Empty);
        board.set_piece(Pos::new(0, 5), Piece::Empty);

        board.set_piece(Pos::new(1, 0), piece1);
        board.set_piece(Pos::new(1, 1), piece1);
        board.set_piece(Pos::new(1, 2), Piece::Empty);
        board.set_piece(Pos::new(1, 3), Piece::Empty);
        board.set_piece(Pos::new(1, 4), Piece::Empty);
        board.set_piece(Pos::new(1, 5), Piece::Empty);

        board.set_piece(Pos::new(2, 0), piece1);
        board.set_piece(Pos::new(2, 1), piece1);
        board.set_piece(Pos::new(2, 2), piece1);
        board.set_piece(Pos::new(2, 3), piece1);
        board.set_piece(Pos::new(2, 4), Piece::Empty);
        board.set_piece(Pos::new(2, 5), Piece::Empty);

        let expected_moves = vec![
            (Pos::new(1, 1), Pos::new(0, 0)),
            (Pos::new(2, 2), Pos::new(1, 1)),
            (Pos::new(2, 3), Pos::new(2, 2))
        ];
        assert_eq!(expected_moves, board.trickle());
    }

    #[test]
    fn trickle_with_diagonals_right_adjacent_uneven_towers_sets_board() {
        let type1 = 'f';
        let piece1 = Piece::Regular(type1, ALL_DIRECTIONS);

        let mut board = Board::new(BoardState::new(16, 16), 
                                   Vec::new(), Vec::new());

        board.set_piece(Pos::new(1, 0), piece1);
        board.set_piece(Pos::new(1, 1), piece1);
        board.set_piece(Pos::new(1, 2), piece1);
        board.set_piece(Pos::new(1, 3), piece1);
        board.set_piece(Pos::new(1, 4), Piece::Empty);
        board.set_piece(Pos::new(1, 5), Piece::Empty);

        board.set_piece(Pos::new(2, 0), piece1);
        board.set_piece(Pos::new(2, 1), piece1);
        board.set_piece(Pos::new(2, 2), Piece::Empty);
        board.set_piece(Pos::new(2, 3), Piece::Empty);
        board.set_piece(Pos::new(2, 4), Piece::Empty);
        board.set_piece(Pos::new(2, 5), Piece::Empty);

        board.set_piece(Pos::new(3, 0), Piece::Empty);
        board.set_piece(Pos::new(3, 1), Piece::Empty);
        board.set_piece(Pos::new(3, 2), Piece::Empty);
        board.set_piece(Pos::new(3, 3), Piece::Empty);
        board.set_piece(Pos::new(3, 4), Piece::Empty);
        board.set_piece(Pos::new(3, 5), Piece::Empty);

        board.trickle();

        assert_eq!(piece1, board.piece(Pos::new(1, 0)));
        assert_eq!(piece1, board.piece(Pos::new(1, 1)));
        assert_eq!(piece1, board.piece(Pos::new(1, 2)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(1, 3)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(1, 4)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(1, 5)));

        assert_eq!(piece1, board.piece(Pos::new(2, 0)));
        assert_eq!(piece1, board.piece(Pos::new(2, 1)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(2, 2)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(2, 3)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(2, 4)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(2, 5)));

        assert_eq!(piece1, board.piece(Pos::new(3, 0)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(3, 1)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(3, 2)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(3, 3)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(3, 4)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(3, 5)));
    }

    #[test]
    fn trickle_with_diagonals_right_adjacent_uneven_towers_generates_moves() {
        let type1 = 'f';
        let piece1 = Piece::Regular(type1, ALL_DIRECTIONS);

        let mut board = Board::new(BoardState::new(16, 16), 
                                   Vec::new(), Vec::new());

        board.set_piece(Pos::new(1, 0), piece1);
        board.set_piece(Pos::new(1, 1), piece1);
        board.set_piece(Pos::new(1, 2), piece1);
        board.set_piece(Pos::new(1, 3), piece1);
        board.set_piece(Pos::new(1, 4), Piece::Empty);
        board.set_piece(Pos::new(1, 5), Piece::Empty);

        board.set_piece(Pos::new(2, 0), piece1);
        board.set_piece(Pos::new(2, 1), piece1);
        board.set_piece(Pos::new(2, 2), Piece::Empty);
        board.set_piece(Pos::new(2, 3), Piece::Empty);
        board.set_piece(Pos::new(2, 4), Piece::Empty);
        board.set_piece(Pos::new(2, 5), Piece::Empty);

        board.set_piece(Pos::new(3, 0), Piece::Empty);
        board.set_piece(Pos::new(3, 1), Piece::Empty);
        board.set_piece(Pos::new(3, 2), Piece::Empty);
        board.set_piece(Pos::new(3, 3), Piece::Empty);
        board.set_piece(Pos::new(3, 4), Piece::Empty);
        board.set_piece(Pos::new(3, 5), Piece::Empty);

        let expected_moves = vec![
            (Pos::new(2, 1), Pos::new(3, 0)),
            (Pos::new(1, 2), Pos::new(2, 1)),
            (Pos::new(1, 3), Pos::new(1, 2))
        ];
        assert_eq!(expected_moves, board.trickle());
    }

    #[test]
    fn trickle_with_diagonals_piece_replaced_with_more_movable_sets_board() {
        let type1 = 'f';
        let piece1 = Piece::Regular(type1, ALL_DIRECTIONS);
        let piece2 = Piece::Regular(type1, enum_set!(Direction::South));

        let mut board = Board::new(BoardState::new(16, 16), 
                                   Vec::new(), Vec::new());

        board.set_piece(Pos::new(1, 0), piece1);
        board.set_piece(Pos::new(1, 1), piece1);
        board.set_piece(Pos::new(1, 2), piece1);
        board.set_piece(Pos::new(1, 3), Piece::Empty);
        board.set_piece(Pos::new(1, 4), Piece::Empty);

        board.set_piece(Pos::new(2, 0), piece1);

        board.set_piece(Pos::new(2, 1), piece2);
        board.set_piece(Pos::new(2, 1), piece1);

        board.set_piece(Pos::new(2, 2), Piece::Empty);
        board.set_piece(Pos::new(2, 3), Piece::Empty);
        board.set_piece(Pos::new(2, 4), Piece::Empty);

        board.set_piece(Pos::new(3, 0), Piece::Empty);
        board.set_piece(Pos::new(3, 1), Piece::Empty);
        board.set_piece(Pos::new(3, 2), Piece::Empty);
        board.set_piece(Pos::new(3, 3), Piece::Empty);
        board.set_piece(Pos::new(3, 4), Piece::Empty);

        board.trickle();

        assert_eq!(piece1, board.piece(Pos::new(1, 0)));
        assert_eq!(piece1, board.piece(Pos::new(1, 1)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(1, 2)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(1, 3)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(1, 4)));

        assert_eq!(piece1, board.piece(Pos::new(2, 0)));
        assert_eq!(piece1, board.piece(Pos::new(2, 1)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(2, 2)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(2, 3)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(2, 4)));

        assert_eq!(piece1, board.piece(Pos::new(3, 0)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(3, 1)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(3, 2)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(3, 3)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(3, 4)));
    }

    #[test]
    fn trickle_with_diagonals_piece_replaced_with_more_movable_generates_moves() {
        let type1 = 'f';
        let piece1 = Piece::Regular(type1, ALL_DIRECTIONS);
        let piece2 = Piece::Regular(type1, enum_set!(Direction::South));

        let mut board = Board::new(BoardState::new(16, 16), 
                                   Vec::new(), Vec::new());

        board.set_piece(Pos::new(1, 0), piece1);
        board.set_piece(Pos::new(1, 1), piece1);
        board.set_piece(Pos::new(1, 2), piece1);
        board.set_piece(Pos::new(1, 3), Piece::Empty);
        board.set_piece(Pos::new(1, 4), Piece::Empty);

        board.set_piece(Pos::new(2, 0), piece1);

        board.set_piece(Pos::new(2, 1), piece2);
        board.set_piece(Pos::new(2, 1), piece1);

        board.set_piece(Pos::new(2, 2), Piece::Empty);
        board.set_piece(Pos::new(2, 3), Piece::Empty);
        board.set_piece(Pos::new(2, 4), Piece::Empty);

        board.set_piece(Pos::new(3, 0), Piece::Empty);
        board.set_piece(Pos::new(3, 1), Piece::Empty);
        board.set_piece(Pos::new(3, 2), Piece::Empty);
        board.set_piece(Pos::new(3, 3), Piece::Empty);
        board.set_piece(Pos::new(3, 4), Piece::Empty);

        let expected_moves: Vec<(Pos, Pos)> = vec![
            (Pos::new(2, 1), Pos::new(3, 0)),
            (Pos::new(1, 2), Pos::new(2, 1))
        ];
        assert_eq!(expected_moves, board.trickle());
    }

    #[test]
    fn trickle_with_diagonals_piece_replaced_with_less_movable_sets_board() {
        let type1 = 'f';
        let piece1 = Piece::Regular(type1, ALL_DIRECTIONS);
        let piece2 = Piece::Regular(type1, enum_set!(Direction::South));

        let mut board = Board::new(BoardState::new(16, 16), 
                                   Vec::new(), Vec::new());

        board.set_piece(Pos::new(1, 0), piece1);
        board.set_piece(Pos::new(1, 1), piece1);
        board.set_piece(Pos::new(1, 2), piece1);
        board.set_piece(Pos::new(1, 3), Piece::Empty);
        board.set_piece(Pos::new(1, 4), Piece::Empty);

        board.set_piece(Pos::new(2, 0), piece1);

        board.set_piece(Pos::new(2, 1), piece1);
        board.set_piece(Pos::new(2, 1), piece2);

        board.set_piece(Pos::new(2, 2), Piece::Empty);
        board.set_piece(Pos::new(2, 3), Piece::Empty);
        board.set_piece(Pos::new(2, 4), Piece::Empty);

        board.set_piece(Pos::new(3, 0), Piece::Empty);
        board.set_piece(Pos::new(3, 1), Piece::Empty);
        board.set_piece(Pos::new(3, 2), Piece::Empty);
        board.set_piece(Pos::new(3, 3), Piece::Empty);
        board.set_piece(Pos::new(3, 4), Piece::Empty);

        assert_eq!(piece1, board.piece(Pos::new(1, 0)));
        assert_eq!(piece1, board.piece(Pos::new(1, 1)));
        assert_eq!(piece1, board.piece(Pos::new(1, 2)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(1, 3)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(1, 4)));

        assert_eq!(piece1, board.piece(Pos::new(2, 0)));
        assert_eq!(piece2, board.piece(Pos::new(2, 1)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(2, 2)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(2, 3)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(2, 4)));

        assert_eq!(Piece::Empty, board.piece(Pos::new(3, 0)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(3, 1)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(3, 2)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(3, 3)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(3, 4)));
    }

    #[test]
    fn trickle_with_diagonals_piece_replaced_with_less_movable_generates_moves() {
        let type1 = 'f';
        let piece1 = Piece::Regular(type1, ALL_DIRECTIONS);
        let piece2 = Piece::Regular(type1, enum_set!(Direction::South));

        let mut board = Board::new(BoardState::new(16, 16), 
                                   Vec::new(), Vec::new());

        board.set_piece(Pos::new(1, 0), piece1);
        board.set_piece(Pos::new(1, 1), piece1);
        board.set_piece(Pos::new(1, 2), piece1);
        board.set_piece(Pos::new(1, 3), Piece::Empty);
        board.set_piece(Pos::new(1, 4), Piece::Empty);

        board.set_piece(Pos::new(2, 0), piece1);

        board.set_piece(Pos::new(2, 1), piece1);
        board.set_piece(Pos::new(2, 1), piece2);

        board.set_piece(Pos::new(2, 2), Piece::Empty);
        board.set_piece(Pos::new(2, 3), Piece::Empty);
        board.set_piece(Pos::new(2, 4), Piece::Empty);

        board.set_piece(Pos::new(3, 0), Piece::Empty);
        board.set_piece(Pos::new(3, 1), Piece::Empty);
        board.set_piece(Pos::new(3, 2), Piece::Empty);
        board.set_piece(Pos::new(3, 3), Piece::Empty);
        board.set_piece(Pos::new(3, 4), Piece::Empty);

        let expected_moves: Vec<(Pos, Pos)> = vec![];
        assert_eq!(expected_moves, board.trickle());
    }

    #[test]
    fn add_trickle_no_diagonals_sets_board() {
        let type1 = 'f';
        let piece1 = Piece::Regular(type1, ALL_DIRECTIONS);

        let mut board = Board::new(BoardState::new(16, 16),
                                   Vec::new(), Vec::new());

        board.set_piece(Pos::new(0, 0), piece1);
        board.set_piece(Pos::new(0, 1), Piece::Empty);
        board.set_piece(Pos::new(0, 2), Piece::Empty);
        board.set_piece(Pos::new(0, 3), Piece::Empty);
        board.set_piece(Pos::new(0, 4), Piece::Empty);
        board.set_piece(Pos::new(0, 5), Piece::Empty);

        board.add_and_trickle(Pos::new(0, 4), piece1);

        assert_eq!(piece1, board.piece(Pos::new(0, 0)));
        assert_eq!(piece1, board.piece(Pos::new(0, 1)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(0, 2)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(0, 3)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(0, 4)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(0, 5)));
    }

    #[test]
    fn add_trickle_no_diagonals_generates_moves() {
        let type1 = 'f';
        let piece1 = Piece::Regular(type1, ALL_DIRECTIONS);

        let mut board = Board::new(BoardState::new(16, 16),
                                   Vec::new(), Vec::new());

        board.set_piece(Pos::new(0, 0), piece1);
        board.set_piece(Pos::new(0, 1), Piece::Empty);
        board.set_piece(Pos::new(0, 2), Piece::Empty);
        board.set_piece(Pos::new(0, 3), Piece::Empty);
        board.set_piece(Pos::new(0, 4), Piece::Empty);
        board.set_piece(Pos::new(0, 5), Piece::Empty);

        let expected_moves = vec![
            (Pos::new(0, 4), Pos::new(0, 1))
        ];
        assert_eq!(expected_moves, board.add_and_trickle(Pos::new(0, 4), piece1));
    }

    #[test]
    fn add_trickle_with_diagonals_left_sets_board() {
        let type1 = 'f';
        let piece1 = Piece::Regular(type1, ALL_DIRECTIONS);

        let mut board = Board::new(BoardState::new(16, 16),
                                   Vec::new(), Vec::new());

        board.set_piece(Pos::new(0, 0), piece1);
        board.set_piece(Pos::new(0, 1), Piece::Empty);
        board.set_piece(Pos::new(0, 2), Piece::Empty);
        board.set_piece(Pos::new(0, 3), Piece::Empty);
        board.set_piece(Pos::new(0, 4), Piece::Empty);
        board.set_piece(Pos::new(0, 5), Piece::Empty);

        board.set_piece(Pos::new(1, 0), piece1);
        board.set_piece(Pos::new(1, 1), piece1);
        board.set_piece(Pos::new(1, 2), Piece::Empty);
        board.set_piece(Pos::new(1, 3), Piece::Empty);
        board.set_piece(Pos::new(1, 4), Piece::Empty);
        board.set_piece(Pos::new(1, 5), Piece::Empty);

        board.set_piece(Pos::new(2, 0), piece1);
        board.set_piece(Pos::new(2, 1), piece1);
        board.set_piece(Pos::new(2, 2), Piece::Empty);
        board.set_piece(Pos::new(2, 3), Piece::Empty);
        board.set_piece(Pos::new(2, 4), Piece::Empty);
        board.set_piece(Pos::new(2, 5), Piece::Empty);

        board.add_and_trickle(Pos::new(1, 4), piece1);

        assert_eq!(piece1, board.piece(Pos::new(0, 0)));
        assert_eq!(piece1, board.piece(Pos::new(0, 1)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(0, 2)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(0, 3)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(0, 4)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(0, 5)));

        assert_eq!(piece1, board.piece(Pos::new(1, 0)));
        assert_eq!(piece1, board.piece(Pos::new(1, 1)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(1, 2)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(1, 3)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(1, 4)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(1, 5)));

        assert_eq!(piece1, board.piece(Pos::new(2, 0)));
        assert_eq!(piece1, board.piece(Pos::new(2, 1)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(2, 2)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(2, 3)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(2, 4)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(2, 5)));
    }

    #[test]
    fn add_trickle_with_diagonals_left_generates_moves() {
        let type1 = 'f';
        let piece1 = Piece::Regular(type1, ALL_DIRECTIONS);

        let mut board = Board::new(BoardState::new(16, 16),
                                   Vec::new(), Vec::new());

        board.set_piece(Pos::new(0, 0), piece1);
        board.set_piece(Pos::new(0, 1), Piece::Empty);
        board.set_piece(Pos::new(0, 2), Piece::Empty);
        board.set_piece(Pos::new(0, 3), Piece::Empty);
        board.set_piece(Pos::new(0, 4), Piece::Empty);
        board.set_piece(Pos::new(0, 5), Piece::Empty);

        board.set_piece(Pos::new(1, 0), piece1);
        board.set_piece(Pos::new(1, 1), piece1);
        board.set_piece(Pos::new(1, 2), Piece::Empty);
        board.set_piece(Pos::new(1, 3), Piece::Empty);
        board.set_piece(Pos::new(1, 4), Piece::Empty);
        board.set_piece(Pos::new(1, 5), Piece::Empty);

        board.set_piece(Pos::new(2, 0), piece1);
        board.set_piece(Pos::new(2, 1), piece1);
        board.set_piece(Pos::new(2, 2), Piece::Empty);
        board.set_piece(Pos::new(2, 3), Piece::Empty);
        board.set_piece(Pos::new(2, 4), Piece::Empty);
        board.set_piece(Pos::new(2, 5), Piece::Empty);

        let expected_moves = vec![
            (Pos::new(1, 4), Pos::new(1, 2)),
            (Pos::new(1, 2), Pos::new(0, 1)),
        ];
        assert_eq!(expected_moves, board.add_and_trickle(Pos::new(1, 4), piece1));
    }

    #[test]
    fn add_trickle_with_diagonals_right_sets_board() {
        let type1 = 'f';
        let piece1 = Piece::Regular(type1, ALL_DIRECTIONS);

        let mut board = Board::new(BoardState::new(16, 16),
                                   Vec::new(), Vec::new());

        board.set_piece(Pos::new(0, 0), piece1);
        board.set_piece(Pos::new(0, 1), Piece::Empty);
        board.set_piece(Pos::new(0, 2), Piece::Empty);
        board.set_piece(Pos::new(0, 3), Piece::Empty);
        board.set_piece(Pos::new(0, 4), Piece::Empty);
        board.set_piece(Pos::new(0, 5), Piece::Empty);

        board.set_piece(Pos::new(1, 0), piece1);
        board.set_piece(Pos::new(1, 1), piece1);
        board.set_piece(Pos::new(1, 2), Piece::Empty);
        board.set_piece(Pos::new(1, 3), Piece::Empty);
        board.set_piece(Pos::new(1, 4), Piece::Empty);
        board.set_piece(Pos::new(1, 5), Piece::Empty);

        board.set_piece(Pos::new(2, 0), piece1);
        board.set_piece(Pos::new(2, 1), piece1);
        board.set_piece(Pos::new(2, 2), Piece::Empty);
        board.set_piece(Pos::new(2, 3), Piece::Empty);
        board.set_piece(Pos::new(2, 4), Piece::Empty);
        board.set_piece(Pos::new(2, 5), Piece::Empty);

        board.set_piece(Pos::new(3, 0), piece1);
        board.set_piece(Pos::new(3, 1), Piece::Empty);
        board.set_piece(Pos::new(3, 2), Piece::Empty);
        board.set_piece(Pos::new(3, 3), Piece::Empty);
        board.set_piece(Pos::new(3, 4), Piece::Empty);
        board.set_piece(Pos::new(3, 5), Piece::Empty);

        board.add_and_trickle(Pos::new(2, 4), piece1);

        assert_eq!(piece1, board.piece(Pos::new(0, 0)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(0, 1)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(0, 2)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(0, 3)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(0, 4)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(0, 5)));

        assert_eq!(piece1, board.piece(Pos::new(1, 0)));
        assert_eq!(piece1, board.piece(Pos::new(1, 1)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(1, 2)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(1, 3)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(1, 4)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(1, 5)));

        assert_eq!(piece1, board.piece(Pos::new(2, 0)));
        assert_eq!(piece1, board.piece(Pos::new(2, 1)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(2, 2)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(2, 3)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(2, 4)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(2, 5)));

        assert_eq!(piece1, board.piece(Pos::new(3, 0)));
        assert_eq!(piece1, board.piece(Pos::new(3, 1)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(3, 2)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(3, 3)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(3, 4)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(3, 5)));
    }

    #[test]
    fn add_trickle_with_diagonals_right_generates_moves() {
        let type1 = 'f';
        let piece1 = Piece::Regular(type1, ALL_DIRECTIONS);

        let mut board = Board::new(BoardState::new(16, 16),
                                   Vec::new(), Vec::new());

        board.set_piece(Pos::new(0, 0), piece1);
        board.set_piece(Pos::new(0, 1), Piece::Empty);
        board.set_piece(Pos::new(0, 2), Piece::Empty);
        board.set_piece(Pos::new(0, 3), Piece::Empty);
        board.set_piece(Pos::new(0, 4), Piece::Empty);
        board.set_piece(Pos::new(0, 5), Piece::Empty);

        board.set_piece(Pos::new(1, 0), piece1);
        board.set_piece(Pos::new(1, 1), piece1);
        board.set_piece(Pos::new(1, 2), Piece::Empty);
        board.set_piece(Pos::new(1, 3), Piece::Empty);
        board.set_piece(Pos::new(1, 4), Piece::Empty);
        board.set_piece(Pos::new(1, 5), Piece::Empty);

        board.set_piece(Pos::new(2, 0), piece1);
        board.set_piece(Pos::new(2, 1), piece1);
        board.set_piece(Pos::new(2, 2), Piece::Empty);
        board.set_piece(Pos::new(2, 3), Piece::Empty);
        board.set_piece(Pos::new(2, 4), Piece::Empty);
        board.set_piece(Pos::new(2, 5), Piece::Empty);

        board.set_piece(Pos::new(3, 0), piece1);
        board.set_piece(Pos::new(3, 1), Piece::Empty);
        board.set_piece(Pos::new(3, 2), Piece::Empty);
        board.set_piece(Pos::new(3, 3), Piece::Empty);
        board.set_piece(Pos::new(3, 4), Piece::Empty);
        board.set_piece(Pos::new(3, 5), Piece::Empty);

        let expected_moves = vec![
            (Pos::new(2, 4), Pos::new(2, 2)),
            (Pos::new(2, 2), Pos::new(3, 1))
        ];
        assert_eq!(expected_moves, board.add_and_trickle(Pos::new(2, 4), piece1));
    }

    #[test]
    fn add_trickle_with_diagonals_ambiguous_sets_board_left_preferred() {
        let type1 = 'f';
        let piece1 = Piece::Regular(type1, ALL_DIRECTIONS);

        let mut board = Board::new(BoardState::new(16, 16),
                                   Vec::new(), Vec::new());

        board.set_piece(Pos::new(0, 0), piece1);
        board.set_piece(Pos::new(0, 1), Piece::Empty);
        board.set_piece(Pos::new(0, 2), Piece::Empty);
        board.set_piece(Pos::new(0, 3), Piece::Empty);
        board.set_piece(Pos::new(0, 4), Piece::Empty);
        board.set_piece(Pos::new(0, 5), Piece::Empty);

        board.set_piece(Pos::new(1, 0), piece1);
        board.set_piece(Pos::new(1, 1), Piece::Empty);
        board.set_piece(Pos::new(1, 2), Piece::Empty);
        board.set_piece(Pos::new(1, 3), Piece::Empty);
        board.set_piece(Pos::new(1, 4), Piece::Empty);
        board.set_piece(Pos::new(1, 5), Piece::Empty);

        board.set_piece(Pos::new(2, 0), piece1);
        board.set_piece(Pos::new(2, 1), piece1);
        board.set_piece(Pos::new(2, 2), Piece::Empty);
        board.set_piece(Pos::new(2, 3), Piece::Empty);
        board.set_piece(Pos::new(2, 4), Piece::Empty);
        board.set_piece(Pos::new(2, 5), Piece::Empty);

        board.set_piece(Pos::new(3, 0), piece1);
        board.set_piece(Pos::new(3, 1), Piece::Empty);
        board.set_piece(Pos::new(3, 2), Piece::Empty);
        board.set_piece(Pos::new(3, 3), Piece::Empty);
        board.set_piece(Pos::new(3, 4), Piece::Empty);
        board.set_piece(Pos::new(3, 5), Piece::Empty);

        board.add_and_trickle(Pos::new(2, 4), piece1);

        assert_eq!(piece1, board.piece(Pos::new(0, 0)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(0, 1)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(0, 2)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(0, 3)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(0, 4)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(0, 5)));

        assert_eq!(piece1, board.piece(Pos::new(1, 0)));
        assert_eq!(piece1, board.piece(Pos::new(1, 1)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(1, 2)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(1, 3)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(1, 4)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(1, 5)));

        assert_eq!(piece1, board.piece(Pos::new(2, 0)));
        assert_eq!(piece1, board.piece(Pos::new(2, 1)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(2, 2)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(2, 3)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(2, 4)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(2, 5)));

        assert_eq!(piece1, board.piece(Pos::new(3, 0)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(3, 1)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(3, 2)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(3, 3)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(3, 4)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(3, 5)));
    }

    #[test]
    fn add_trickle_with_diagonals_ambiguous_generates_moves_left_preferred() {
        let type1 = 'f';
        let piece1 = Piece::Regular(type1, ALL_DIRECTIONS);

        let mut board = Board::new(BoardState::new(16, 16),
                                   Vec::new(), Vec::new());

        board.set_piece(Pos::new(0, 0), piece1);
        board.set_piece(Pos::new(0, 1), Piece::Empty);
        board.set_piece(Pos::new(0, 2), Piece::Empty);
        board.set_piece(Pos::new(0, 3), Piece::Empty);
        board.set_piece(Pos::new(0, 4), Piece::Empty);
        board.set_piece(Pos::new(0, 5), Piece::Empty);

        board.set_piece(Pos::new(1, 0), piece1);
        board.set_piece(Pos::new(1, 1), Piece::Empty);
        board.set_piece(Pos::new(1, 2), Piece::Empty);
        board.set_piece(Pos::new(1, 3), Piece::Empty);
        board.set_piece(Pos::new(1, 4), Piece::Empty);
        board.set_piece(Pos::new(1, 5), Piece::Empty);

        board.set_piece(Pos::new(2, 0), piece1);
        board.set_piece(Pos::new(2, 1), piece1);
        board.set_piece(Pos::new(2, 2), Piece::Empty);
        board.set_piece(Pos::new(2, 3), Piece::Empty);
        board.set_piece(Pos::new(2, 4), Piece::Empty);
        board.set_piece(Pos::new(2, 5), Piece::Empty);

        board.set_piece(Pos::new(3, 0), piece1);
        board.set_piece(Pos::new(3, 1), Piece::Empty);
        board.set_piece(Pos::new(3, 2), Piece::Empty);
        board.set_piece(Pos::new(3, 3), Piece::Empty);
        board.set_piece(Pos::new(3, 4), Piece::Empty);
        board.set_piece(Pos::new(3, 5), Piece::Empty);

        let expected_moves = vec![
            (Pos::new(2, 4), Pos::new(2, 2)),
            (Pos::new(2, 2), Pos::new(1, 1))
        ];
        assert_eq!(expected_moves, board.add_and_trickle(Pos::new(2, 4), piece1));
    }

    #[test]
    fn add_trickle_with_diagonals_blocking_wall_sets_board() {
        let type1 = 'f';
        let piece1 = Piece::Regular(type1, ALL_DIRECTIONS);

        let mut board = Board::new(BoardState::new(16, 16),
                                   Vec::new(), Vec::new());

        board.set_piece(Pos::new(0, 0), Piece::Empty);
        board.set_piece(Pos::new(0, 1), Piece::Empty);
        board.set_piece(Pos::new(0, 2), Piece::Empty);
        board.set_piece(Pos::new(0, 3), Piece::Wall);
        board.set_piece(Pos::new(0, 4), piece1);
        board.set_piece(Pos::new(0, 5), Piece::Empty);

        board.set_piece(Pos::new(1, 0), piece1);
        board.set_piece(Pos::new(1, 1), Piece::Empty);
        board.set_piece(Pos::new(1, 2), Piece::Empty);
        board.set_piece(Pos::new(1, 3), Piece::Wall);
        board.set_piece(Pos::new(1, 4), Piece::Empty);
        board.set_piece(Pos::new(1, 5), Piece::Empty);

        board.set_piece(Pos::new(2, 0), Piece::Empty);
        board.set_piece(Pos::new(2, 1), Piece::Empty);
        board.set_piece(Pos::new(2, 2), Piece::Empty);
        board.set_piece(Pos::new(2, 3), Piece::Wall);
        board.set_piece(Pos::new(2, 4), piece1);
        board.set_piece(Pos::new(2, 5), piece1);

        board.set_piece(Pos::new(3, 0), piece1);
        board.set_piece(Pos::new(3, 1), Piece::Empty);
        board.set_piece(Pos::new(3, 2), Piece::Empty);
        board.set_piece(Pos::new(3, 3), Piece::Wall);
        board.set_piece(Pos::new(3, 4), Piece::Empty);
        board.set_piece(Pos::new(3, 5), Piece::Empty);

        board.add_and_trickle(Pos::new(1, 5), piece1);

        assert_eq!(Piece::Empty, board.piece(Pos::new(0, 0)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(0, 1)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(0, 2)));
        assert_eq!(Piece::Wall, board.piece(Pos::new(0, 3)));
        assert_eq!(piece1, board.piece(Pos::new(0, 4)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(0, 5)));

        assert_eq!(piece1, board.piece(Pos::new(1, 0)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(1, 1)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(1, 2)));
        assert_eq!(Piece::Wall, board.piece(Pos::new(1, 3)));
        assert_eq!(piece1, board.piece(Pos::new(1, 4)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(1, 5)));

        assert_eq!(Piece::Empty, board.piece(Pos::new(2, 0)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(2, 1)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(2, 2)));
        assert_eq!(Piece::Wall, board.piece(Pos::new(2, 3)));
        assert_eq!(piece1, board.piece(Pos::new(2, 4)));
        assert_eq!(piece1, board.piece(Pos::new(2, 5)));

        assert_eq!(piece1, board.piece(Pos::new(3, 0)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(3, 1)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(3, 2)));
        assert_eq!(Piece::Wall, board.piece(Pos::new(3, 3)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(3, 4)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(3, 5)));
    }

    #[test]
    fn add_trickle_with_diagonals_blocking_wall_generates_moves() {
        let type1 = 'f';
        let piece1 = Piece::Regular(type1, ALL_DIRECTIONS);

        let mut board = Board::new(BoardState::new(16, 16),
                                   Vec::new(), Vec::new());

        board.set_piece(Pos::new(0, 0), Piece::Empty);
        board.set_piece(Pos::new(0, 1), Piece::Empty);
        board.set_piece(Pos::new(0, 2), Piece::Empty);
        board.set_piece(Pos::new(0, 3), Piece::Wall);
        board.set_piece(Pos::new(0, 4), piece1);
        board.set_piece(Pos::new(0, 5), Piece::Empty);

        board.set_piece(Pos::new(1, 0), piece1);
        board.set_piece(Pos::new(1, 1), Piece::Empty);
        board.set_piece(Pos::new(1, 2), Piece::Empty);
        board.set_piece(Pos::new(1, 3), Piece::Wall);
        board.set_piece(Pos::new(1, 4), Piece::Empty);
        board.set_piece(Pos::new(1, 5), Piece::Empty);

        board.set_piece(Pos::new(2, 0), Piece::Empty);
        board.set_piece(Pos::new(2, 1), Piece::Empty);
        board.set_piece(Pos::new(2, 2), Piece::Empty);
        board.set_piece(Pos::new(2, 3), Piece::Wall);
        board.set_piece(Pos::new(2, 4), piece1);
        board.set_piece(Pos::new(2, 5), piece1);

        board.set_piece(Pos::new(3, 0), piece1);
        board.set_piece(Pos::new(3, 1), Piece::Empty);
        board.set_piece(Pos::new(3, 2), Piece::Empty);
        board.set_piece(Pos::new(3, 3), Piece::Wall);
        board.set_piece(Pos::new(3, 4), Piece::Empty);
        board.set_piece(Pos::new(3, 5), Piece::Empty);

        let expected_moves = vec![
            (Pos::new(1, 5), Pos::new(1, 4))
        ];
        assert_eq!(expected_moves, board.add_and_trickle(Pos::new(1, 5), piece1));
    }

    #[test]
    fn add_trickle_with_diagonals_through_hole_sets_board() {
        let type1 = 'f';
        let piece1 = Piece::Regular(type1, ALL_DIRECTIONS);

        let mut board = Board::new(BoardState::new(16, 16),
                                   Vec::new(), Vec::new());

        board.set_piece(Pos::new(0, 0), Piece::Empty);
        board.set_piece(Pos::new(0, 1), Piece::Empty);
        board.set_piece(Pos::new(0, 2), Piece::Empty);
        board.set_piece(Pos::new(0, 3), Piece::Empty);
        board.set_piece(Pos::new(0, 4), Piece::Wall);
        board.set_piece(Pos::new(0, 5), piece1);

        board.set_piece(Pos::new(1, 0), Piece::Empty);
        board.set_piece(Pos::new(1, 1), Piece::Empty);
        board.set_piece(Pos::new(1, 2), Piece::Empty);
        board.set_piece(Pos::new(1, 3), Piece::Empty);
        board.set_piece(Pos::new(1, 4), Piece::Wall);
        board.set_piece(Pos::new(1, 5), Piece::Empty);

        board.set_piece(Pos::new(2, 0), piece1);
        board.set_piece(Pos::new(2, 1), Piece::Empty);
        board.set_piece(Pos::new(2, 2), Piece::Empty);
        board.set_piece(Pos::new(2, 3), Piece::Empty);
        board.set_piece(Pos::new(2, 4), Piece::Empty);
        board.set_piece(Pos::new(2, 5), Piece::Empty);

        board.set_piece(Pos::new(3, 0), Piece::Empty);
        board.set_piece(Pos::new(3, 1), Piece::Empty);
        board.set_piece(Pos::new(3, 2), Piece::Empty);
        board.set_piece(Pos::new(3, 3), Piece::Empty);
        board.set_piece(Pos::new(3, 4), Piece::Wall);
        board.set_piece(Pos::new(3, 5), Piece::Empty);

        board.add_and_trickle(Pos::new(1, 5), piece1);

        assert_eq!(Piece::Empty, board.piece(Pos::new(0, 0)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(0, 1)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(0, 2)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(0, 3)));
        assert_eq!(Piece::Wall, board.piece(Pos::new(0, 4)));
        assert_eq!(piece1, board.piece(Pos::new(0, 5)));

        assert_eq!(piece1, board.piece(Pos::new(1, 0)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(1, 1)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(1, 2)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(1, 3)));
        assert_eq!(Piece::Wall, board.piece(Pos::new(1, 4)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(1, 5)));

        assert_eq!(piece1, board.piece(Pos::new(2, 0)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(2, 1)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(2, 2)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(2, 3)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(2, 4)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(2, 5)));

        assert_eq!(Piece::Empty, board.piece(Pos::new(3, 0)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(3, 1)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(3, 2)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(3, 3)));
        assert_eq!(Piece::Wall, board.piece(Pos::new(3, 4)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(3, 5)));
    }

    #[test]
    fn add_trickle_with_diagonals_through_hole_generates_moves() {
        let type1 = 'f';
        let piece1 = Piece::Regular(type1, ALL_DIRECTIONS);

        let mut board = Board::new(BoardState::new(16, 16),
                                   Vec::new(), Vec::new());

        board.set_piece(Pos::new(0, 0), Piece::Empty);
        board.set_piece(Pos::new(0, 1), Piece::Empty);
        board.set_piece(Pos::new(0, 2), Piece::Empty);
        board.set_piece(Pos::new(0, 3), Piece::Empty);
        board.set_piece(Pos::new(0, 4), Piece::Wall);
        board.set_piece(Pos::new(0, 5), piece1);

        board.set_piece(Pos::new(1, 0), Piece::Empty);
        board.set_piece(Pos::new(1, 1), Piece::Empty);
        board.set_piece(Pos::new(1, 2), Piece::Empty);
        board.set_piece(Pos::new(1, 3), Piece::Empty);
        board.set_piece(Pos::new(1, 4), Piece::Wall);
        board.set_piece(Pos::new(1, 5), Piece::Empty);

        board.set_piece(Pos::new(2, 0), piece1);
        board.set_piece(Pos::new(2, 1), Piece::Empty);
        board.set_piece(Pos::new(2, 2), Piece::Empty);
        board.set_piece(Pos::new(2, 3), Piece::Empty);
        board.set_piece(Pos::new(2, 4), Piece::Empty);
        board.set_piece(Pos::new(2, 5), Piece::Empty);

        board.set_piece(Pos::new(3, 0), Piece::Empty);
        board.set_piece(Pos::new(3, 1), Piece::Empty);
        board.set_piece(Pos::new(3, 2), Piece::Empty);
        board.set_piece(Pos::new(3, 3), Piece::Empty);
        board.set_piece(Pos::new(3, 4), Piece::Wall);
        board.set_piece(Pos::new(3, 5), Piece::Empty);

        let expected_moves = vec![
            (Pos::new(1, 5), Pos::new(2, 4)),
            (Pos::new(2, 4), Pos::new(2, 1)),
            (Pos::new(2, 1), Pos::new(1, 0))
        ];
        assert_eq!(expected_moves, board.add_and_trickle(Pos::new(1, 5), piece1));
    }

    #[test]
    fn add_trickle_with_diagonals_changing_directions_sets_board() {
        let type1 = 'f';
        let piece1 = Piece::Regular(type1, ALL_DIRECTIONS);

        let mut board = Board::new(BoardState::new(16, 16),
                                   Vec::new(), Vec::new());

        board.set_piece(Pos::new(0, 0), piece1);
        board.set_piece(Pos::new(0, 1), Piece::Empty);
        board.set_piece(Pos::new(0, 2), Piece::Empty);
        board.set_piece(Pos::new(0, 3), Piece::Empty);
        board.set_piece(Pos::new(0, 4), Piece::Wall);
        board.set_piece(Pos::new(0, 5), piece1);

        board.set_piece(Pos::new(1, 0), piece1);
        board.set_piece(Pos::new(1, 1), piece1);
        board.set_piece(Pos::new(1, 2), Piece::Empty);
        board.set_piece(Pos::new(1, 3), Piece::Empty);
        board.set_piece(Pos::new(1, 4), Piece::Wall);
        board.set_piece(Pos::new(1, 5), Piece::Empty);

        board.set_piece(Pos::new(2, 0), piece1);
        board.set_piece(Pos::new(2, 1), Piece::Empty);
        board.set_piece(Pos::new(2, 2), Piece::Empty);
        board.set_piece(Pos::new(2, 3), Piece::Empty);
        board.set_piece(Pos::new(2, 4), Piece::Empty);
        board.set_piece(Pos::new(2, 5), Piece::Empty);

        board.set_piece(Pos::new(3, 0), Piece::Empty);
        board.set_piece(Pos::new(3, 1), Piece::Empty);
        board.set_piece(Pos::new(3, 2), Piece::Empty);
        board.set_piece(Pos::new(3, 3), Piece::Empty);
        board.set_piece(Pos::new(3, 4), Piece::Wall);
        board.set_piece(Pos::new(3, 5), Piece::Empty);

        board.add_and_trickle(Pos::new(3, 5), piece1);

        assert_eq!(piece1, board.piece(Pos::new(0, 0)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(0, 1)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(0, 2)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(0, 3)));
        assert_eq!(Piece::Wall, board.piece(Pos::new(0, 4)));
        assert_eq!(piece1, board.piece(Pos::new(0, 5)));

        assert_eq!(piece1, board.piece(Pos::new(1, 0)));
        assert_eq!(piece1, board.piece(Pos::new(1, 1)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(1, 2)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(1, 3)));
        assert_eq!(Piece::Wall, board.piece(Pos::new(1, 4)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(1, 5)));

        assert_eq!(piece1, board.piece(Pos::new(2, 0)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(2, 1)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(2, 2)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(2, 3)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(2, 4)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(2, 5)));

        assert_eq!(piece1, board.piece(Pos::new(3, 0)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(3, 1)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(3, 2)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(3, 3)));
        assert_eq!(Piece::Wall, board.piece(Pos::new(3, 4)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(3, 5)));
    }

    #[test]
    fn add_trickle_with_diagonals_changing_directions_generates_moves() {
        let type1 = 'f';
        let piece1 = Piece::Regular(type1, ALL_DIRECTIONS);

        let mut board = Board::new(BoardState::new(16, 16),
                                   Vec::new(), Vec::new());

        board.set_piece(Pos::new(0, 0), piece1);
        board.set_piece(Pos::new(0, 1), Piece::Empty);
        board.set_piece(Pos::new(0, 2), Piece::Empty);
        board.set_piece(Pos::new(0, 3), Piece::Empty);
        board.set_piece(Pos::new(0, 4), Piece::Wall);
        board.set_piece(Pos::new(0, 5), piece1);

        board.set_piece(Pos::new(1, 0), piece1);
        board.set_piece(Pos::new(1, 1), piece1);
        board.set_piece(Pos::new(1, 2), Piece::Empty);
        board.set_piece(Pos::new(1, 3), Piece::Empty);
        board.set_piece(Pos::new(1, 4), Piece::Wall);
        board.set_piece(Pos::new(1, 5), Piece::Empty);

        board.set_piece(Pos::new(2, 0), piece1);
        board.set_piece(Pos::new(2, 1), Piece::Empty);
        board.set_piece(Pos::new(2, 2), Piece::Empty);
        board.set_piece(Pos::new(2, 3), Piece::Empty);
        board.set_piece(Pos::new(2, 4), Piece::Empty);
        board.set_piece(Pos::new(2, 5), Piece::Empty);

        board.set_piece(Pos::new(3, 0), Piece::Empty);
        board.set_piece(Pos::new(3, 1), Piece::Empty);
        board.set_piece(Pos::new(3, 2), Piece::Empty);
        board.set_piece(Pos::new(3, 3), Piece::Empty);
        board.set_piece(Pos::new(3, 4), Piece::Wall);
        board.set_piece(Pos::new(3, 5), Piece::Empty);

        let expected_moves = vec![
            (Pos::new(3, 5), Pos::new(2, 4)),
            (Pos::new(2, 4), Pos::new(2, 1)),
            (Pos::new(2, 1), Pos::new(3, 0))
        ];
        assert_eq!(expected_moves, board.add_and_trickle(Pos::new(3, 5), piece1));
    }

    #[test]
    fn add_trickle_with_diagonals_west_wall_sets_board() {
        let type1 = 'f';
        let piece1 = Piece::Regular(type1, ALL_DIRECTIONS);

        let mut board = Board::new(BoardState::new(16, 16),
                                   Vec::new(), Vec::new());

        board.set_piece(Pos::new(0, 0), piece1);
        board.set_piece(Pos::new(0, 1), Piece::Empty);
        board.set_piece(Pos::new(0, 2), Piece::Wall);
        board.set_piece(Pos::new(0, 3), Piece::Empty);
        board.set_piece(Pos::new(0, 4), Piece::Empty);
        board.set_piece(Pos::new(0, 5), Piece::Empty);

        board.set_piece(Pos::new(1, 0), piece1);
        board.set_piece(Pos::new(1, 1), piece1);
        board.set_piece(Pos::new(1, 2), Piece::Empty);
        board.set_piece(Pos::new(1, 3), Piece::Empty);
        board.set_piece(Pos::new(1, 4), Piece::Empty);
        board.set_piece(Pos::new(1, 5), Piece::Empty);

        board.set_piece(Pos::new(2, 0), piece1);
        board.set_piece(Pos::new(2, 1), piece1);
        board.set_piece(Pos::new(2, 2), Piece::Empty);
        board.set_piece(Pos::new(2, 3), Piece::Empty);
        board.set_piece(Pos::new(2, 4), Piece::Empty);
        board.set_piece(Pos::new(2, 5), Piece::Empty);

        board.set_piece(Pos::new(3, 0), piece1);
        board.set_piece(Pos::new(3, 1), Piece::Empty);
        board.set_piece(Pos::new(3, 2), Piece::Empty);
        board.set_piece(Pos::new(3, 3), Piece::Empty);
        board.set_piece(Pos::new(3, 4), Piece::Empty);
        board.set_piece(Pos::new(3, 5), Piece::Empty);

        board.add_and_trickle(Pos::new(1, 5), piece1);

        assert_eq!(piece1, board.piece(Pos::new(0, 0)));
        assert_eq!(piece1, board.piece(Pos::new(0, 1)));
        assert_eq!(Piece::Wall, board.piece(Pos::new(0, 2)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(0, 3)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(0, 4)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(0, 5)));

        assert_eq!(piece1, board.piece(Pos::new(1, 0)));
        assert_eq!(piece1, board.piece(Pos::new(1, 1)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(1, 2)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(1, 3)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(1, 4)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(1, 5)));

        assert_eq!(piece1, board.piece(Pos::new(2, 0)));
        assert_eq!(piece1, board.piece(Pos::new(2, 1)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(2, 2)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(2, 3)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(2, 4)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(2, 5)));

        assert_eq!(piece1, board.piece(Pos::new(3, 0)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(3, 1)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(3, 2)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(3, 3)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(3, 4)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(3, 5)));
    }

    #[test]
    fn add_trickle_with_diagonals_west_wall_generates_moves() {
        let type1 = 'f';
        let piece1 = Piece::Regular(type1, ALL_DIRECTIONS);

        let mut board = Board::new(BoardState::new(16, 16),
                                   Vec::new(), Vec::new());

        board.set_piece(Pos::new(0, 0), piece1);
        board.set_piece(Pos::new(0, 1), Piece::Empty);
        board.set_piece(Pos::new(0, 2), Piece::Wall);
        board.set_piece(Pos::new(0, 3), Piece::Empty);
        board.set_piece(Pos::new(0, 4), Piece::Empty);
        board.set_piece(Pos::new(0, 5), Piece::Empty);

        board.set_piece(Pos::new(1, 0), piece1);
        board.set_piece(Pos::new(1, 1), piece1);
        board.set_piece(Pos::new(1, 2), Piece::Empty);
        board.set_piece(Pos::new(1, 3), Piece::Empty);
        board.set_piece(Pos::new(1, 4), Piece::Empty);
        board.set_piece(Pos::new(1, 5), Piece::Empty);

        board.set_piece(Pos::new(2, 0), piece1);
        board.set_piece(Pos::new(2, 1), piece1);
        board.set_piece(Pos::new(2, 2), Piece::Empty);
        board.set_piece(Pos::new(2, 3), Piece::Empty);
        board.set_piece(Pos::new(2, 4), Piece::Empty);
        board.set_piece(Pos::new(2, 5), Piece::Empty);

        board.set_piece(Pos::new(3, 0), piece1);
        board.set_piece(Pos::new(3, 1), Piece::Empty);
        board.set_piece(Pos::new(3, 2), Piece::Empty);
        board.set_piece(Pos::new(3, 3), Piece::Empty);
        board.set_piece(Pos::new(3, 4), Piece::Empty);
        board.set_piece(Pos::new(3, 5), Piece::Empty);

        let expected_moves = vec![
            (Pos::new(1, 5), Pos::new(1, 2)),
            (Pos::new(1, 2), Pos::new(0, 1))
        ];
        assert_eq!(expected_moves, board.add_and_trickle(Pos::new(1, 5), piece1));
    }

    #[test]
    fn add_trickle_with_diagonals_east_wall_sets_board() {
        let type1 = 'f';
        let piece1 = Piece::Regular(type1, ALL_DIRECTIONS);

        let mut board = Board::new(BoardState::new(16, 16),
                                   Vec::new(), Vec::new());

        board.set_piece(Pos::new(0, 0), piece1);
        board.set_piece(Pos::new(0, 1), Piece::Empty);
        board.set_piece(Pos::new(0, 2), Piece::Empty);
        board.set_piece(Pos::new(0, 3), Piece::Empty);
        board.set_piece(Pos::new(0, 4), Piece::Empty);
        board.set_piece(Pos::new(0, 5), Piece::Empty);

        board.set_piece(Pos::new(1, 0), piece1);
        board.set_piece(Pos::new(1, 1), piece1);
        board.set_piece(Pos::new(1, 2), Piece::Empty);
        board.set_piece(Pos::new(1, 3), Piece::Empty);
        board.set_piece(Pos::new(1, 4), Piece::Empty);
        board.set_piece(Pos::new(1, 5), Piece::Empty);

        board.set_piece(Pos::new(2, 0), piece1);
        board.set_piece(Pos::new(2, 1), piece1);
        board.set_piece(Pos::new(2, 2), Piece::Empty);
        board.set_piece(Pos::new(2, 3), Piece::Empty);
        board.set_piece(Pos::new(2, 4), Piece::Empty);
        board.set_piece(Pos::new(2, 5), Piece::Empty);

        board.set_piece(Pos::new(3, 0), piece1);
        board.set_piece(Pos::new(3, 1), Piece::Empty);
        board.set_piece(Pos::new(3, 2), Piece::Wall);
        board.set_piece(Pos::new(3, 3), Piece::Empty);
        board.set_piece(Pos::new(3, 4), Piece::Empty);
        board.set_piece(Pos::new(3, 5), Piece::Empty);

        board.add_and_trickle(Pos::new(2, 5), piece1);

        assert_eq!(piece1, board.piece(Pos::new(0, 0)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(0, 1)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(0, 2)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(0, 3)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(0, 4)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(0, 5)));

        assert_eq!(piece1, board.piece(Pos::new(1, 0)));
        assert_eq!(piece1, board.piece(Pos::new(1, 1)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(1, 2)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(1, 3)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(1, 4)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(1, 5)));

        assert_eq!(piece1, board.piece(Pos::new(2, 0)));
        assert_eq!(piece1, board.piece(Pos::new(2, 1)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(2, 2)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(2, 3)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(2, 4)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(2, 5)));

        assert_eq!(piece1, board.piece(Pos::new(3, 0)));
        assert_eq!(piece1, board.piece(Pos::new(3, 1)));
        assert_eq!(Piece::Wall, board.piece(Pos::new(3, 2)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(3, 3)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(3, 4)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(3, 5)));
    }

    #[test]
    fn add_trickle_with_diagonals_east_wall_generates_moves() {
        let type1 = 'f';
        let piece1 = Piece::Regular(type1, ALL_DIRECTIONS);

        let mut board = Board::new(BoardState::new(16, 16),
                                   Vec::new(), Vec::new());

        board.set_piece(Pos::new(0, 0), piece1);
        board.set_piece(Pos::new(0, 1), Piece::Empty);
        board.set_piece(Pos::new(0, 2), Piece::Empty);
        board.set_piece(Pos::new(0, 3), Piece::Empty);
        board.set_piece(Pos::new(0, 4), Piece::Empty);
        board.set_piece(Pos::new(0, 5), Piece::Empty);

        board.set_piece(Pos::new(1, 0), piece1);
        board.set_piece(Pos::new(1, 1), piece1);
        board.set_piece(Pos::new(1, 2), Piece::Empty);
        board.set_piece(Pos::new(1, 3), Piece::Empty);
        board.set_piece(Pos::new(1, 4), Piece::Empty);
        board.set_piece(Pos::new(1, 5), Piece::Empty);

        board.set_piece(Pos::new(2, 0), piece1);
        board.set_piece(Pos::new(2, 1), piece1);
        board.set_piece(Pos::new(2, 2), Piece::Empty);
        board.set_piece(Pos::new(2, 3), Piece::Empty);
        board.set_piece(Pos::new(2, 4), Piece::Empty);
        board.set_piece(Pos::new(2, 5), Piece::Empty);

        board.set_piece(Pos::new(3, 0), piece1);
        board.set_piece(Pos::new(3, 1), Piece::Empty);
        board.set_piece(Pos::new(3, 2), Piece::Wall);
        board.set_piece(Pos::new(3, 3), Piece::Empty);
        board.set_piece(Pos::new(3, 4), Piece::Empty);
        board.set_piece(Pos::new(3, 5), Piece::Empty);

        let expected_moves = vec![
            (Pos::new(2, 5), Pos::new(2, 2)),
            (Pos::new(2, 2), Pos::new(3, 1))
        ];
        assert_eq!(expected_moves, board.add_and_trickle(Pos::new(2, 5), piece1));
    }

    #[test]
    fn add_trickle_no_diagonals_unmovable_north_sets_board() {
        let type1 = 'f';
        let piece1 = Piece::Regular(type1, ALL_DIRECTIONS);
        let piece2 = Piece::Regular(type1, enum_set!(
            Direction::South | Direction::East | Direction::West
        ));

        let mut board = Board::new(BoardState::new(16, 16),
                                   Vec::new(), Vec::new());

        board.set_piece(Pos::new(1, 0), piece1);
        board.set_piece(Pos::new(1, 1), Piece::Empty);
        board.set_piece(Pos::new(1, 2), Piece::Empty);
        board.set_piece(Pos::new(1, 3), Piece::Empty);
        board.set_piece(Pos::new(1, 4), Piece::Empty);
        board.set_piece(Pos::new(1, 5), Piece::Empty);

        board.add_and_trickle(Pos::new(1, 2), piece2);

        assert_eq!(piece1, board.piece(Pos::new(1, 0)));
        assert_eq!(piece2, board.piece(Pos::new(1, 1)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(1, 2)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(1, 3)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(1, 4)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(1, 5)));
    }

    #[test]
    fn add_trickle_no_diagonals_unmovable_north_generates_moves() {
        let type1 = 'f';
        let piece1 = Piece::Regular(type1, ALL_DIRECTIONS);
        let piece2 = Piece::Regular(type1, enum_set!(
            Direction::South | Direction::East | Direction::West
        ));

        let mut board = Board::new(BoardState::new(16, 16),
                                   Vec::new(), Vec::new());

        board.set_piece(Pos::new(1, 0), piece1);
        board.set_piece(Pos::new(1, 1), Piece::Empty);
        board.set_piece(Pos::new(1, 2), Piece::Empty);
        board.set_piece(Pos::new(1, 3), Piece::Empty);
        board.set_piece(Pos::new(1, 4), Piece::Empty);
        board.set_piece(Pos::new(1, 5), Piece::Empty);

        let expected_moves = vec![
            (Pos::new(1, 2), Pos::new(1, 1))
        ];
        assert_eq!(expected_moves, board.add_and_trickle(Pos::new(1, 2), piece2));
    }

    #[test]
    fn add_trickle_no_diagonals_unmovable_south_sets_board_for_movable() {
        let type1 = 'f';
        let piece1 = Piece::Regular(type1, ALL_DIRECTIONS);
        let piece2 = Piece::Regular(type1, enum_set!(
            Direction::North | Direction::East | Direction::West
        ));

        let mut board = Board::new(BoardState::new(16, 16),
                                   Vec::new(), Vec::new());

        board.set_piece(Pos::new(1, 0), piece1);
        board.set_piece(Pos::new(1, 1), Piece::Empty);
        board.set_piece(Pos::new(1, 2), Piece::Empty);
        board.set_piece(Pos::new(1, 3), Piece::Empty);
        board.set_piece(Pos::new(1, 4), Piece::Empty);
        board.set_piece(Pos::new(1, 5), Piece::Empty);

        board.add_and_trickle(Pos::new(1, 2), piece2);

        assert_eq!(piece1, board.piece(Pos::new(1, 0)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(1, 1)));
        assert_eq!(piece2, board.piece(Pos::new(1, 2)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(1, 3)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(1, 4)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(1, 5)));
    }

    #[test]
    fn add_trickle_no_diagonals_unmovable_south_generates_moves_for_movable() {
        let type1 = 'f';
        let piece1 = Piece::Regular(type1, ALL_DIRECTIONS);
        let piece2 = Piece::Regular(type1, enum_set!(
            Direction::North | Direction::East | Direction::West
        ));

        let mut board = Board::new(BoardState::new(16, 16),
                                   Vec::new(), Vec::new());

        board.set_piece(Pos::new(1, 0), piece1);
        board.set_piece(Pos::new(1, 1), Piece::Empty);
        board.set_piece(Pos::new(1, 2), Piece::Empty);
        board.set_piece(Pos::new(1, 3), Piece::Empty);
        board.set_piece(Pos::new(1, 4), Piece::Empty);
        board.set_piece(Pos::new(1, 5), Piece::Empty);

        let expected_moves: Vec<(Pos, Pos)> = vec![];
        assert_eq!(expected_moves, board.add_and_trickle(Pos::new(1, 2), piece2));
    }

    #[test]
    fn add_trickle_no_diagonals_unmovable_east_sets_board() {
        let type1 = 'f';
        let piece1 = Piece::Regular(type1, ALL_DIRECTIONS);
        let piece2 = Piece::Regular(type1, enum_set!(
            Direction::South | Direction::North | Direction::West
        ));

        let mut board = Board::new(BoardState::new(16, 16),
                                   Vec::new(), Vec::new());

        board.set_piece(Pos::new(1, 0), piece1);
        board.set_piece(Pos::new(1, 1), Piece::Empty);
        board.set_piece(Pos::new(1, 2), Piece::Empty);
        board.set_piece(Pos::new(1, 3), Piece::Empty);
        board.set_piece(Pos::new(1, 4), Piece::Empty);
        board.set_piece(Pos::new(1, 5), Piece::Empty);

        board.add_and_trickle(Pos::new(1, 2), piece2);

        assert_eq!(piece1, board.piece(Pos::new(1, 0)));
        assert_eq!(piece2, board.piece(Pos::new(1, 1)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(1, 2)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(1, 3)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(1, 4)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(1, 5)));
    }

    #[test]
    fn add_trickle_no_diagonals_unmovable_east_generates_moves() {
        let type1 = 'f';
        let piece1 = Piece::Regular(type1, ALL_DIRECTIONS);
        let piece2 = Piece::Regular(type1, enum_set!(
            Direction::South | Direction::North | Direction::West
        ));

        let mut board = Board::new(BoardState::new(16, 16),
                                   Vec::new(), Vec::new());

        board.set_piece(Pos::new(1, 0), piece1);
        board.set_piece(Pos::new(1, 1), Piece::Empty);
        board.set_piece(Pos::new(1, 2), Piece::Empty);
        board.set_piece(Pos::new(1, 3), Piece::Empty);
        board.set_piece(Pos::new(1, 4), Piece::Empty);
        board.set_piece(Pos::new(1, 5), Piece::Empty);

        let expected_moves = vec![
            (Pos::new(1, 2), Pos::new(1, 1))
        ];
        assert_eq!(expected_moves, board.add_and_trickle(Pos::new(1, 2), piece2));
    }

    #[test]
    fn add_trickle_no_diagonals_unmovable_west_sets_board() {
        let type1 = 'f';
        let piece1 = Piece::Regular(type1, ALL_DIRECTIONS);
        let piece2 = Piece::Regular(type1, enum_set!(
            Direction::South | Direction::North | Direction::East
        ));

        let mut board = Board::new(BoardState::new(16, 16),
                                   Vec::new(), Vec::new());

        board.set_piece(Pos::new(1, 0), piece1);
        board.set_piece(Pos::new(1, 1), Piece::Empty);
        board.set_piece(Pos::new(1, 2), Piece::Empty);
        board.set_piece(Pos::new(1, 3), Piece::Empty);
        board.set_piece(Pos::new(1, 4), Piece::Empty);
        board.set_piece(Pos::new(1, 5), Piece::Empty);

        board.add_and_trickle(Pos::new(1, 2), piece2);

        assert_eq!(piece1, board.piece(Pos::new(1, 0)));
        assert_eq!(piece2, board.piece(Pos::new(1, 1)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(1, 2)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(1, 3)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(1, 4)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(1, 5)));
    }

    #[test]
    fn add_trickle_no_diagonals_unmovable_west_generates_moves() {
        let type1 = 'f';
        let piece1 = Piece::Regular(type1, ALL_DIRECTIONS);
        let piece2 = Piece::Regular(type1, enum_set!(
            Direction::South | Direction::North | Direction::East
        ));

        let mut board = Board::new(BoardState::new(16, 16),
                                   Vec::new(), Vec::new());

        board.set_piece(Pos::new(1, 0), piece1);
        board.set_piece(Pos::new(1, 1), Piece::Empty);
        board.set_piece(Pos::new(1, 2), Piece::Empty);
        board.set_piece(Pos::new(1, 3), Piece::Empty);
        board.set_piece(Pos::new(1, 4), Piece::Empty);
        board.set_piece(Pos::new(1, 5), Piece::Empty);

        let expected_moves = vec![
            (Pos::new(1, 2), Pos::new(1, 1))
        ];
        assert_eq!(expected_moves, board.add_and_trickle(Pos::new(1, 2), piece2));
    }

    #[test]
    fn add_trickle_with_diagonals_left_unmovable_north_sets_board() {
        let type1 = 'f';
        let piece1 = Piece::Regular(type1, ALL_DIRECTIONS);
        let piece2 = Piece::Regular(type1, enum_set!(
            Direction::South | Direction::East | Direction::West
        ));

        let mut board = Board::new(BoardState::new(16, 16),
                                   Vec::new(), Vec::new());

        board.set_piece(Pos::new(1, 0), piece1);
        board.set_piece(Pos::new(1, 1), Piece::Empty);
        board.set_piece(Pos::new(1, 2), Piece::Empty);
        board.set_piece(Pos::new(1, 3), Piece::Empty);
        board.set_piece(Pos::new(1, 4), Piece::Empty);
        board.set_piece(Pos::new(1, 5), Piece::Empty);

        board.set_piece(Pos::new(0, 0), Piece::Empty);
        board.set_piece(Pos::new(0, 1), Piece::Empty);
        board.set_piece(Pos::new(0, 2), Piece::Empty);
        board.set_piece(Pos::new(0, 3), Piece::Empty);
        board.set_piece(Pos::new(0, 4), Piece::Empty);
        board.set_piece(Pos::new(0, 5), Piece::Empty);

        board.add_and_trickle(Pos::new(1, 2), piece2);

        assert_eq!(piece2, board.piece(Pos::new(0, 0)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(0, 1)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(0, 2)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(0, 3)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(0, 4)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(0, 5)));

        assert_eq!(piece1, board.piece(Pos::new(1, 0)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(1, 1)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(1, 2)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(1, 3)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(1, 4)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(1, 5)));
    }

    #[test]
    fn add_trickle_with_diagonals_left_unmovable_north_generates_moves() {
        let type1 = 'f';
        let piece1 = Piece::Regular(type1, ALL_DIRECTIONS);
        let piece2 = Piece::Regular(type1, enum_set!(
            Direction::South | Direction::East | Direction::West
        ));

        let mut board = Board::new(BoardState::new(16, 16),
                                   Vec::new(), Vec::new());

        board.set_piece(Pos::new(0, 0), Piece::Empty);
        board.set_piece(Pos::new(0, 1), Piece::Empty);
        board.set_piece(Pos::new(0, 2), Piece::Empty);
        board.set_piece(Pos::new(0, 3), Piece::Empty);
        board.set_piece(Pos::new(0, 4), Piece::Empty);
        board.set_piece(Pos::new(0, 5), Piece::Empty);

        board.set_piece(Pos::new(1, 0), piece1);
        board.set_piece(Pos::new(1, 1), Piece::Empty);
        board.set_piece(Pos::new(1, 2), Piece::Empty);
        board.set_piece(Pos::new(1, 3), Piece::Empty);
        board.set_piece(Pos::new(1, 4), Piece::Empty);
        board.set_piece(Pos::new(1, 5), Piece::Empty);

        let expected_moves = vec![
            (Pos::new(1, 2), Pos::new(1, 1)),
            (Pos::new(1, 1), Pos::new(0, 0))
        ];
        assert_eq!(expected_moves, board.add_and_trickle(Pos::new(1, 2), piece2));
    }

    #[test]
    fn add_trickle_with_diagonals_left_unmovable_south_sets_board() {
        let type1 = 'f';
        let piece1 = Piece::Regular(type1, ALL_DIRECTIONS);
        let piece2 = Piece::Regular(type1, enum_set!(
            Direction::North | Direction::East | Direction::West
        ));

        let mut board = Board::new(BoardState::new(16, 16),
                                   Vec::new(), Vec::new());

        board.set_piece(Pos::new(0, 0), Piece::Empty);
        board.set_piece(Pos::new(0, 1), Piece::Empty);
        board.set_piece(Pos::new(0, 2), Piece::Empty);
        board.set_piece(Pos::new(0, 3), Piece::Empty);
        board.set_piece(Pos::new(0, 4), Piece::Empty);
        board.set_piece(Pos::new(0, 5), Piece::Empty);

        board.set_piece(Pos::new(1, 0), piece1);
        board.set_piece(Pos::new(1, 1), Piece::Empty);
        board.set_piece(Pos::new(1, 2), Piece::Empty);
        board.set_piece(Pos::new(1, 3), Piece::Empty);
        board.set_piece(Pos::new(1, 4), Piece::Empty);
        board.set_piece(Pos::new(1, 5), Piece::Empty);

        board.add_and_trickle(Pos::new(1, 2), piece2);

        assert_eq!(piece1, board.piece(Pos::new(1, 0)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(1, 1)));
        assert_eq!(piece2, board.piece(Pos::new(1, 2)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(1, 3)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(1, 4)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(1, 5)));

        assert_eq!(Piece::Empty, board.piece(Pos::new(0, 0)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(0, 1)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(0, 2)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(0, 3)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(0, 4)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(0, 5)));
    }

    #[test]
    fn add_trickle_with_diagonals_left_unmovable_south_generates_moves() {
        let type1 = 'f';
        let piece1 = Piece::Regular(type1, ALL_DIRECTIONS);
        let piece2 = Piece::Regular(type1, enum_set!(
            Direction::North | Direction::East | Direction::West
        ));

        let mut board = Board::new(BoardState::new(16, 16),
                                   Vec::new(), Vec::new());

        board.set_piece(Pos::new(0, 0), Piece::Empty);
        board.set_piece(Pos::new(0, 1), Piece::Empty);
        board.set_piece(Pos::new(0, 2), Piece::Empty);
        board.set_piece(Pos::new(0, 3), Piece::Empty);
        board.set_piece(Pos::new(0, 4), Piece::Empty);
        board.set_piece(Pos::new(0, 5), Piece::Empty);

        board.set_piece(Pos::new(1, 0), piece1);
        board.set_piece(Pos::new(1, 1), Piece::Empty);
        board.set_piece(Pos::new(1, 2), piece2);
        board.set_piece(Pos::new(1, 3), Piece::Empty);
        board.set_piece(Pos::new(1, 4), Piece::Empty);
        board.set_piece(Pos::new(1, 5), Piece::Empty);

        let expected_moves: Vec<(Pos, Pos)> = vec![];
        assert_eq!(expected_moves, board.add_and_trickle(Pos::new(1, 2), piece2));
    }

    #[test]
    fn add_trickle_with_diagonals_left_unmovable_east_sets_board() {
        let type1 = 'f';
        let piece1 = Piece::Regular(type1, ALL_DIRECTIONS);
        let piece2 = Piece::Regular(type1, enum_set!(
            Direction::South | Direction::North | Direction::West
        ));

        let mut board = Board::new(BoardState::new(16, 16),
                                   Vec::new(), Vec::new());

        board.set_piece(Pos::new(0, 0), Piece::Empty);
        board.set_piece(Pos::new(0, 1), Piece::Empty);
        board.set_piece(Pos::new(0, 2), Piece::Empty);
        board.set_piece(Pos::new(0, 3), Piece::Empty);
        board.set_piece(Pos::new(0, 4), Piece::Empty);
        board.set_piece(Pos::new(0, 5), Piece::Empty);

        board.set_piece(Pos::new(1, 0), piece1);
        board.set_piece(Pos::new(1, 1), Piece::Empty);
        board.set_piece(Pos::new(1, 2), Piece::Empty);
        board.set_piece(Pos::new(1, 3), Piece::Empty);
        board.set_piece(Pos::new(1, 4), Piece::Empty);
        board.set_piece(Pos::new(1, 5), Piece::Empty);

        board.add_and_trickle(Pos::new(1, 2), piece2);

        assert_eq!(piece2, board.piece(Pos::new(0, 0)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(0, 1)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(0, 2)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(0, 3)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(0, 4)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(0, 5)));

        assert_eq!(piece1, board.piece(Pos::new(1, 0)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(1, 1)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(1, 2)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(1, 3)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(1, 4)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(1, 5)));
    }

    #[test]
    fn add_trickle_with_diagonals_left_unmovable_east_generates_moves() {
        let type1 = 'f';
        let piece1 = Piece::Regular(type1, ALL_DIRECTIONS);
        let piece2 = Piece::Regular(type1, enum_set!(
            Direction::South | Direction::North | Direction::West
        ));

        let mut board = Board::new(BoardState::new(16, 16),
                                   Vec::new(), Vec::new());
        board.set_piece(Pos::new(0, 0), Piece::Empty);
        board.set_piece(Pos::new(0, 1), Piece::Empty);
        board.set_piece(Pos::new(0, 2), Piece::Empty);
        board.set_piece(Pos::new(0, 3), Piece::Empty);
        board.set_piece(Pos::new(0, 4), Piece::Empty);
        board.set_piece(Pos::new(0, 5), Piece::Empty);

        board.set_piece(Pos::new(1, 0), piece1);
        board.set_piece(Pos::new(1, 1), Piece::Empty);
        board.set_piece(Pos::new(1, 2), Piece::Empty);
        board.set_piece(Pos::new(1, 3), Piece::Empty);
        board.set_piece(Pos::new(1, 4), Piece::Empty);
        board.set_piece(Pos::new(1, 5), Piece::Empty);

        let expected_moves = vec![
            (Pos::new(1, 2), Pos::new(1, 1)),
            (Pos::new(1, 1), Pos::new(0, 0))
        ];
        assert_eq!(expected_moves, board.add_and_trickle(Pos::new(1, 2), piece2));
    }

    #[test]
    fn add_trickle_with_diagonals_left_unmovable_west_sets_board() {
        let type1 = 'f';
        let piece1 = Piece::Regular(type1, ALL_DIRECTIONS);
        let piece2 = Piece::Regular(type1, enum_set!(
            Direction::South | Direction::East | Direction::North
        ));

        let mut board = Board::new(BoardState::new(16, 16),
                                   Vec::new(), Vec::new());

        board.set_piece(Pos::new(0, 0), Piece::Empty);
        board.set_piece(Pos::new(0, 1), Piece::Empty);
        board.set_piece(Pos::new(0, 2), Piece::Empty);
        board.set_piece(Pos::new(0, 3), Piece::Empty);
        board.set_piece(Pos::new(0, 4), Piece::Empty);
        board.set_piece(Pos::new(0, 5), Piece::Empty);

        board.set_piece(Pos::new(1, 0), piece1);
        board.set_piece(Pos::new(1, 1), Piece::Empty);
        board.set_piece(Pos::new(1, 2), Piece::Empty);
        board.set_piece(Pos::new(1, 3), Piece::Empty);
        board.set_piece(Pos::new(1, 4), Piece::Empty);
        board.set_piece(Pos::new(1, 5), Piece::Empty);

        board.add_and_trickle(Pos::new(1, 2), piece2);

        assert_eq!(Piece::Empty, board.piece(Pos::new(0, 0)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(0, 1)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(0, 2)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(0, 3)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(0, 4)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(0, 5)));

        assert_eq!(piece1, board.piece(Pos::new(1, 0)));
        assert_eq!(piece2, board.piece(Pos::new(1, 1)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(1, 2)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(1, 3)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(1, 4)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(1, 5)));
    }

    #[test]
    fn add_trickle_with_diagonals_left_unmovable_west_generates_moves() {
        let type1 = 'f';
        let piece1 = Piece::Regular(type1, ALL_DIRECTIONS);
        let piece2 = Piece::Regular(type1, enum_set!(
            Direction::South | Direction::East | Direction::North
        ));

        let mut board = Board::new(BoardState::new(16, 16),
                                   Vec::new(), Vec::new());

        board.set_piece(Pos::new(0, 0), Piece::Empty);
        board.set_piece(Pos::new(0, 1), Piece::Empty);
        board.set_piece(Pos::new(0, 2), Piece::Empty);
        board.set_piece(Pos::new(0, 3), Piece::Empty);
        board.set_piece(Pos::new(0, 4), Piece::Empty);
        board.set_piece(Pos::new(0, 5), Piece::Empty);

        board.set_piece(Pos::new(1, 0), piece1);
        board.set_piece(Pos::new(1, 1), Piece::Empty);
        board.set_piece(Pos::new(1, 2), Piece::Empty);
        board.set_piece(Pos::new(1, 3), Piece::Empty);
        board.set_piece(Pos::new(1, 4), Piece::Empty);
        board.set_piece(Pos::new(1, 5), Piece::Empty);

        let expected_moves = vec![
            (Pos::new(1, 2), Pos::new(1, 1))
        ];
        assert_eq!(expected_moves, board.add_and_trickle(Pos::new(1, 2), piece2));
    }

    #[test]
    fn add_trickle_with_diagonals_right_unmovable_north_sets_board() {
        let type1 = 'f';
        let piece1 = Piece::Regular(type1, ALL_DIRECTIONS);
        let piece2 = Piece::Regular(type1, enum_set!(
            Direction::South | Direction::East | Direction::West
        ));

        let mut board = Board::new(BoardState::new(16, 16),
                                   Vec::new(), Vec::new());

        board.set_piece(Pos::new(1, 0), piece1);
        board.set_piece(Pos::new(1, 1), Piece::Empty);
        board.set_piece(Pos::new(1, 2), Piece::Empty);
        board.set_piece(Pos::new(1, 3), Piece::Empty);
        board.set_piece(Pos::new(1, 4), Piece::Empty);
        board.set_piece(Pos::new(1, 5), Piece::Empty);

        board.set_piece(Pos::new(2, 0), Piece::Empty);
        board.set_piece(Pos::new(2, 1), Piece::Empty);
        board.set_piece(Pos::new(2, 2), Piece::Empty);
        board.set_piece(Pos::new(2, 3), Piece::Empty);
        board.set_piece(Pos::new(2, 4), Piece::Empty);
        board.set_piece(Pos::new(2, 5), Piece::Empty);

        board.add_and_trickle(Pos::new(1, 2), piece2);

        assert_eq!(piece1, board.piece(Pos::new(1, 0)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(1, 1)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(1, 2)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(1, 3)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(1, 4)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(1, 5)));

        assert_eq!(piece2, board.piece(Pos::new(2, 0)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(2, 1)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(2, 2)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(2, 3)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(2, 4)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(2, 5)));
    }

    #[test]
    fn add_trickle_with_diagonals_right_unmovable_north_generates_moves() {
        let type1 = 'f';
        let piece1 = Piece::Regular(type1, ALL_DIRECTIONS);
        let piece2 = Piece::Regular(type1, enum_set!(
            Direction::South | Direction::East | Direction::West
        ));

        let mut board = Board::new(BoardState::new(16, 16),
                                   Vec::new(), Vec::new());

        board.set_piece(Pos::new(1, 0), piece1);
        board.set_piece(Pos::new(1, 1), Piece::Empty);
        board.set_piece(Pos::new(1, 2), Piece::Empty);
        board.set_piece(Pos::new(1, 3), Piece::Empty);
        board.set_piece(Pos::new(1, 4), Piece::Empty);
        board.set_piece(Pos::new(1, 5), Piece::Empty);

        board.set_piece(Pos::new(2, 0), Piece::Empty);
        board.set_piece(Pos::new(2, 1), Piece::Empty);
        board.set_piece(Pos::new(2, 2), Piece::Empty);
        board.set_piece(Pos::new(2, 3), Piece::Empty);
        board.set_piece(Pos::new(2, 4), Piece::Empty);
        board.set_piece(Pos::new(2, 5), Piece::Empty);

        let expected_moves = vec![
            (Pos::new(1, 2), Pos::new(1, 1)),
            (Pos::new(1, 1), Pos::new(2, 0))
        ];
        assert_eq!(expected_moves, board.add_and_trickle(Pos::new(1, 2), piece2));
    }

    #[test]
    fn add_trickle_with_diagonals_right_unmovable_south_sets_board() {
        let type1 = 'f';
        let piece1 = Piece::Regular(type1, ALL_DIRECTIONS);
        let piece2 = Piece::Regular(type1, enum_set!(
            Direction::North | Direction::East | Direction::West
        ));

        let mut board = Board::new(BoardState::new(16, 16),
                                   Vec::new(), Vec::new());

        board.set_piece(Pos::new(1, 0), piece1);
        board.set_piece(Pos::new(1, 1), Piece::Empty);
        board.set_piece(Pos::new(1, 2), Piece::Empty);
        board.set_piece(Pos::new(1, 3), Piece::Empty);
        board.set_piece(Pos::new(1, 4), Piece::Empty);
        board.set_piece(Pos::new(1, 5), Piece::Empty);

        board.set_piece(Pos::new(2, 0), Piece::Empty);
        board.set_piece(Pos::new(2, 1), Piece::Empty);
        board.set_piece(Pos::new(2, 2), Piece::Empty);
        board.set_piece(Pos::new(2, 3), Piece::Empty);
        board.set_piece(Pos::new(2, 4), Piece::Empty);
        board.set_piece(Pos::new(2, 5), Piece::Empty);

        board.add_and_trickle(Pos::new(1, 2), piece2);

        assert_eq!(piece1, board.piece(Pos::new(1, 0)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(1, 1)));
        assert_eq!(piece2, board.piece(Pos::new(1, 2)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(1, 3)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(1, 4)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(1, 5)));

        assert_eq!(Piece::Empty, board.piece(Pos::new(2, 0)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(2, 1)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(2, 2)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(2, 3)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(2, 4)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(2, 5)));
    }

    #[test]
    fn add_trickle_with_diagonals_right_unmovable_south_generates_moves() {
        let type1 = 'f';
        let piece1 = Piece::Regular(type1, ALL_DIRECTIONS);
        let piece2 = Piece::Regular(type1, enum_set!(
            Direction::North | Direction::East | Direction::West
        ));

        let mut board = Board::new(BoardState::new(16, 16),
                                   Vec::new(), Vec::new());

        board.set_piece(Pos::new(1, 0), piece1);
        board.set_piece(Pos::new(1, 1), Piece::Empty);
        board.set_piece(Pos::new(1, 2), Piece::Empty);
        board.set_piece(Pos::new(1, 3), Piece::Empty);
        board.set_piece(Pos::new(1, 4), Piece::Empty);
        board.set_piece(Pos::new(1, 5), Piece::Empty);

        board.set_piece(Pos::new(2, 0), Piece::Empty);
        board.set_piece(Pos::new(2, 1), Piece::Empty);
        board.set_piece(Pos::new(2, 2), Piece::Empty);
        board.set_piece(Pos::new(2, 3), Piece::Empty);
        board.set_piece(Pos::new(2, 4), Piece::Empty);
        board.set_piece(Pos::new(2, 5), Piece::Empty);

        let expected_moves: Vec<(Pos, Pos)> = vec![];
        assert_eq!(expected_moves, board.add_and_trickle(Pos::new(1, 2), piece2));
    }

    #[test]
    fn add_trickle_with_diagonals_right_unmovable_east_sets_board() {
        let type1 = 'f';
        let piece1 = Piece::Regular(type1, ALL_DIRECTIONS);
        let piece2 = Piece::Regular(type1, enum_set!(
            Direction::South | Direction::North | Direction::West
        ));

        let mut board = Board::new(BoardState::new(16, 16),
                                   Vec::new(), Vec::new());

        board.set_piece(Pos::new(1, 0), piece1);
        board.set_piece(Pos::new(1, 1), Piece::Empty);
        board.set_piece(Pos::new(1, 2), Piece::Empty);
        board.set_piece(Pos::new(1, 3), Piece::Empty);
        board.set_piece(Pos::new(1, 4), Piece::Empty);
        board.set_piece(Pos::new(1, 5), Piece::Empty);

        board.set_piece(Pos::new(2, 0), Piece::Empty);
        board.set_piece(Pos::new(2, 1), Piece::Empty);
        board.set_piece(Pos::new(2, 2), Piece::Empty);
        board.set_piece(Pos::new(2, 3), Piece::Empty);
        board.set_piece(Pos::new(2, 4), Piece::Empty);
        board.set_piece(Pos::new(2, 5), Piece::Empty);

        board.add_and_trickle(Pos::new(1, 2), piece2);

        assert_eq!(piece1, board.piece(Pos::new(1, 0)));
        assert_eq!(piece2, board.piece(Pos::new(1, 1)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(1, 2)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(1, 3)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(1, 4)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(1, 5)));

        assert_eq!(Piece::Empty, board.piece(Pos::new(2, 0)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(2, 1)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(2, 2)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(2, 3)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(2, 4)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(2, 5)));
    }

    #[test]
    fn add_trickle_with_diagonals_right_unmovable_east_generates_moves() {
        let type1 = 'f';
        let piece1 = Piece::Regular(type1, ALL_DIRECTIONS);
        let piece2 = Piece::Regular(type1, enum_set!(
            Direction::South | Direction::North | Direction::West
        ));

        let mut board = Board::new(BoardState::new(16, 16),
                                   Vec::new(), Vec::new());

        board.set_piece(Pos::new(1, 0), piece1);
        board.set_piece(Pos::new(1, 1), Piece::Empty);
        board.set_piece(Pos::new(1, 2), Piece::Empty);
        board.set_piece(Pos::new(1, 3), Piece::Empty);
        board.set_piece(Pos::new(1, 4), Piece::Empty);
        board.set_piece(Pos::new(1, 5), Piece::Empty);

        board.set_piece(Pos::new(2, 0), Piece::Empty);
        board.set_piece(Pos::new(2, 1), Piece::Empty);
        board.set_piece(Pos::new(2, 2), Piece::Empty);
        board.set_piece(Pos::new(2, 3), Piece::Empty);
        board.set_piece(Pos::new(2, 4), Piece::Empty);
        board.set_piece(Pos::new(2, 5), Piece::Empty);

        let expected_moves = vec![
            (Pos::new(1, 2), Pos::new(1, 1))
        ];
        assert_eq!(expected_moves, board.add_and_trickle(Pos::new(1, 2), piece2));
    }

    #[test]
    fn add_trickle_with_diagonals_right_unmovable_west_sets_board() {
        let type1 = 'f';
        let piece1 = Piece::Regular(type1, ALL_DIRECTIONS);
        let piece2 = Piece::Regular(type1, enum_set!(
            Direction::South | Direction::East | Direction::North
        ));

        let mut board = Board::new(BoardState::new(16, 16),
                                   Vec::new(), Vec::new());

        board.set_piece(Pos::new(1, 0), piece1);
        board.set_piece(Pos::new(1, 1), Piece::Empty);
        board.set_piece(Pos::new(1, 2), Piece::Empty);
        board.set_piece(Pos::new(1, 3), Piece::Empty);
        board.set_piece(Pos::new(1, 4), Piece::Empty);
        board.set_piece(Pos::new(1, 5), Piece::Empty);

        board.set_piece(Pos::new(2, 0), Piece::Empty);
        board.set_piece(Pos::new(2, 1), Piece::Empty);
        board.set_piece(Pos::new(2, 2), Piece::Empty);
        board.set_piece(Pos::new(2, 3), Piece::Empty);
        board.set_piece(Pos::new(2, 4), Piece::Empty);
        board.set_piece(Pos::new(2, 5), Piece::Empty);

        board.add_and_trickle(Pos::new(1, 2), piece2);

        assert_eq!(piece1, board.piece(Pos::new(1, 0)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(1, 1)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(1, 2)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(1, 3)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(1, 4)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(1, 5)));

        assert_eq!(piece2, board.piece(Pos::new(2, 0)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(2, 1)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(2, 2)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(2, 3)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(2, 4)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(2, 5)));
    }

    #[test]
    fn add_trickle_with_diagonals_right_unmovable_west_generates_moves() {
        let type1 = 'f';
        let piece1 = Piece::Regular(type1, ALL_DIRECTIONS);
        let piece2 = Piece::Regular(type1, enum_set!(
            Direction::South | Direction::East | Direction::North
        ));

        let mut board = Board::new(BoardState::new(16, 16),
                                   Vec::new(), Vec::new());

        board.set_piece(Pos::new(1, 0), piece1);
        board.set_piece(Pos::new(1, 1), Piece::Empty);
        board.set_piece(Pos::new(1, 2), Piece::Empty);
        board.set_piece(Pos::new(1, 3), Piece::Empty);
        board.set_piece(Pos::new(1, 4), Piece::Empty);
        board.set_piece(Pos::new(1, 5), Piece::Empty);

        board.set_piece(Pos::new(2, 0), Piece::Empty);
        board.set_piece(Pos::new(2, 1), Piece::Empty);
        board.set_piece(Pos::new(2, 2), Piece::Empty);
        board.set_piece(Pos::new(2, 3), Piece::Empty);
        board.set_piece(Pos::new(2, 4), Piece::Empty);
        board.set_piece(Pos::new(2, 5), Piece::Empty);

        let expected_moves = vec![
            (Pos::new(1, 2), Pos::new(1, 1)),
            (Pos::new(1, 1), Pos::new(2, 0))
        ];
        assert_eq!(expected_moves, board.add_and_trickle(Pos::new(1, 2), piece2));
    }

    #[test]
    fn add_trickle_right_border_sets_board() {
        let type1 = 'f';
        let piece1 = Piece::Regular(type1, ALL_DIRECTIONS);

        let mut board = Board::new(BoardState::new(16, 16),
                                   Vec::new(), Vec::new());

        board.set_piece(Pos::new(15, 0), piece1);
        board.set_piece(Pos::new(15, 1), Piece::Empty);
        board.set_piece(Pos::new(15, 2), Piece::Empty);
        board.set_piece(Pos::new(15, 3), Piece::Empty);
        board.set_piece(Pos::new(15, 4), Piece::Empty);
        board.set_piece(Pos::new(15, 5), Piece::Empty);

        board.add_and_trickle(Pos::new(15, 2), piece1);

        assert_eq!(piece1, board.piece(Pos::new(15, 0)));
        assert_eq!(piece1, board.piece(Pos::new(15, 1)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(15, 2)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(15, 3)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(15, 4)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(15, 5)));
    }

    #[test]
    fn add_trickle_right_border_generates_moves() {
        let type1 = 'f';
        let piece1 = Piece::Regular(type1, ALL_DIRECTIONS);

        let mut board = Board::new(BoardState::new(16, 16),
                                   Vec::new(), Vec::new());

        board.set_piece(Pos::new(15, 0), piece1);
        board.set_piece(Pos::new(15, 1), Piece::Empty);
        board.set_piece(Pos::new(15, 2), Piece::Empty);
        board.set_piece(Pos::new(15, 3), Piece::Empty);
        board.set_piece(Pos::new(15, 4), Piece::Empty);
        board.set_piece(Pos::new(15, 5), Piece::Empty);

        let expected_moves = vec![
            (Pos::new(15, 2), Pos::new(15, 1))
        ];
        assert_eq!(expected_moves, board.add_and_trickle(Pos::new(15, 2), piece1));
    }

    #[test]
    fn add_trickle_top_border_sets_board() {
        let type1 = 'f';
        let piece1 = Piece::Regular(type1, ALL_DIRECTIONS);

        let mut board = Board::new(BoardState::new(16, 16),
                                   Vec::new(), Vec::new());

        board.set_piece(Pos::new(0, 10), piece1);
        board.set_piece(Pos::new(0, 11), Piece::Empty);
        board.set_piece(Pos::new(0, 12), Piece::Empty);
        board.set_piece(Pos::new(0, 13), Piece::Empty);
        board.set_piece(Pos::new(0, 14), Piece::Empty);
        board.set_piece(Pos::new(0, 15), Piece::Empty);

        board.add_and_trickle(Pos::new(0, 12), piece1);

        assert_eq!(piece1, board.piece(Pos::new(0, 10)));
        assert_eq!(piece1, board.piece(Pos::new(0, 11)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(0, 12)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(0, 13)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(0, 14)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(0, 15)));
    }

    #[test]
    fn add_trickle_top_border_generates_moves() {
        let type1 = 'f';
        let piece1 = Piece::Regular(type1, ALL_DIRECTIONS);

        let mut board = Board::new(BoardState::new(16, 16),
                                   Vec::new(), Vec::new());

        board.set_piece(Pos::new(0, 10), piece1);
        board.set_piece(Pos::new(0, 11), Piece::Empty);
        board.set_piece(Pos::new(0, 12), Piece::Empty);
        board.set_piece(Pos::new(0, 13), Piece::Empty);
        board.set_piece(Pos::new(0, 14), Piece::Empty);
        board.set_piece(Pos::new(0, 15), Piece::Empty);

        let expected_moves = vec![
            (Pos::new(0, 12), Pos::new(0, 11))
        ];
        assert_eq!(expected_moves, board.add_and_trickle(Pos::new(0, 12), piece1));
    }

    #[test]
    fn add_trickle_with_diagonals_piece_replaced_with_more_movable_sets_board() {
        let type1 = 'f';
        let piece1 = Piece::Regular(type1, ALL_DIRECTIONS);
        let piece2 = Piece::Regular(type1, enum_set!(Direction::South));

        let mut board = Board::new(BoardState::new(16, 16),
                                   Vec::new(), Vec::new());

        board.set_piece(Pos::new(1, 0), piece1);
        board.set_piece(Pos::new(1, 1), piece1);
        board.set_piece(Pos::new(1, 2), piece1);
        board.set_piece(Pos::new(1, 3), Piece::Empty);
        board.set_piece(Pos::new(1, 4), Piece::Empty);

        board.set_piece(Pos::new(2, 0), piece1);

        board.set_piece(Pos::new(2, 1), piece2);

        board.set_piece(Pos::new(2, 2), Piece::Empty);
        board.set_piece(Pos::new(2, 3), Piece::Empty);
        board.set_piece(Pos::new(2, 4), Piece::Empty);

        board.set_piece(Pos::new(3, 0), Piece::Empty);
        board.set_piece(Pos::new(3, 1), Piece::Empty);
        board.set_piece(Pos::new(3, 2), Piece::Empty);
        board.set_piece(Pos::new(3, 3), Piece::Empty);
        board.set_piece(Pos::new(3, 4), Piece::Empty);

        board.add_and_trickle(Pos::new(2, 1), piece1);

        assert_eq!(piece1, board.piece(Pos::new(1, 0)));
        assert_eq!(piece1, board.piece(Pos::new(1, 1)));
        assert_eq!(piece1, board.piece(Pos::new(1, 2)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(1, 3)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(1, 4)));

        assert_eq!(piece1, board.piece(Pos::new(2, 0)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(2, 1)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(2, 2)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(2, 3)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(2, 4)));

        assert_eq!(piece1, board.piece(Pos::new(3, 0)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(3, 1)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(3, 2)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(3, 3)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(3, 4)));
    }

    #[test]
    fn add_trickle_with_diagonals_piece_replaced_with_more_movable_generates_moves() {
        let type1 = 'f';
        let piece1 = Piece::Regular(type1, ALL_DIRECTIONS);
        let piece2 = Piece::Regular(type1, enum_set!(Direction::South));

        let mut board = Board::new(BoardState::new(16, 16),
                                   Vec::new(), Vec::new());

        board.set_piece(Pos::new(1, 0), piece1);
        board.set_piece(Pos::new(1, 1), piece1);
        board.set_piece(Pos::new(1, 2), piece1);
        board.set_piece(Pos::new(1, 3), Piece::Empty);
        board.set_piece(Pos::new(1, 4), Piece::Empty);

        board.set_piece(Pos::new(2, 0), piece1);

        board.set_piece(Pos::new(2, 1), piece2);

        board.set_piece(Pos::new(2, 2), Piece::Empty);
        board.set_piece(Pos::new(2, 3), Piece::Empty);
        board.set_piece(Pos::new(2, 4), Piece::Empty);

        board.set_piece(Pos::new(3, 0), Piece::Empty);
        board.set_piece(Pos::new(3, 1), Piece::Empty);
        board.set_piece(Pos::new(3, 2), Piece::Empty);
        board.set_piece(Pos::new(3, 3), Piece::Empty);
        board.set_piece(Pos::new(3, 4), Piece::Empty);

        let expected_moves: Vec<(Pos, Pos)> = vec![
            (Pos::new(2, 1), Pos::new(3, 0))
        ];
        assert_eq!(expected_moves, board.add_and_trickle(Pos::new(2, 1), piece1));
    }

    #[test]
    fn add_trickle_with_diagonals_piece_replaced_with_less_movable_sets_board() {
        let type1 = 'f';
        let piece1 = Piece::Regular(type1, ALL_DIRECTIONS);
        let piece2 = Piece::Regular(type1, enum_set!(Direction::South));

        let mut board = Board::new(BoardState::new(16, 16),
                                   Vec::new(), Vec::new());

        board.set_piece(Pos::new(1, 0), piece1);
        board.set_piece(Pos::new(1, 1), piece1);
        board.set_piece(Pos::new(1, 2), piece1);
        board.set_piece(Pos::new(1, 3), Piece::Empty);
        board.set_piece(Pos::new(1, 4), Piece::Empty);

        board.set_piece(Pos::new(2, 0), piece1);

        board.set_piece(Pos::new(2, 1), piece1);

        board.set_piece(Pos::new(2, 2), Piece::Empty);
        board.set_piece(Pos::new(2, 3), Piece::Empty);
        board.set_piece(Pos::new(2, 4), Piece::Empty);

        board.set_piece(Pos::new(3, 0), Piece::Empty);
        board.set_piece(Pos::new(3, 1), Piece::Empty);
        board.set_piece(Pos::new(3, 2), Piece::Empty);
        board.set_piece(Pos::new(3, 3), Piece::Empty);
        board.set_piece(Pos::new(3, 4), Piece::Empty);

        board.add_and_trickle(Pos::new(2, 1), piece2);

        assert_eq!(piece1, board.piece(Pos::new(1, 0)));
        assert_eq!(piece1, board.piece(Pos::new(1, 1)));
        assert_eq!(piece1, board.piece(Pos::new(1, 2)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(1, 3)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(1, 4)));

        assert_eq!(piece1, board.piece(Pos::new(2, 0)));
        assert_eq!(piece2, board.piece(Pos::new(2, 1)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(2, 2)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(2, 3)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(2, 4)));

        assert_eq!(Piece::Empty, board.piece(Pos::new(3, 0)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(3, 1)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(3, 2)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(3, 3)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(3, 4)));
    }

    #[test]
    fn add_trickle_with_diagonals_piece_replaced_with_less_movable_generates_moves() {
        let type1 = 'f';
        let piece1 = Piece::Regular(type1, ALL_DIRECTIONS);
        let piece2 = Piece::Regular(type1, enum_set!(Direction::South));

        let mut board = Board::new(BoardState::new(16, 16),
                                   Vec::new(), Vec::new());

        board.set_piece(Pos::new(1, 0), piece1);
        board.set_piece(Pos::new(1, 1), piece1);
        board.set_piece(Pos::new(1, 2), piece1);
        board.set_piece(Pos::new(1, 3), Piece::Empty);
        board.set_piece(Pos::new(1, 4), Piece::Empty);

        board.set_piece(Pos::new(2, 0), piece1);

        board.set_piece(Pos::new(2, 1), piece1);

        board.set_piece(Pos::new(2, 2), Piece::Empty);
        board.set_piece(Pos::new(2, 3), Piece::Empty);
        board.set_piece(Pos::new(2, 4), Piece::Empty);

        board.set_piece(Pos::new(3, 0), Piece::Empty);
        board.set_piece(Pos::new(3, 1), Piece::Empty);
        board.set_piece(Pos::new(3, 2), Piece::Empty);
        board.set_piece(Pos::new(3, 3), Piece::Empty);
        board.set_piece(Pos::new(3, 4), Piece::Empty);

        let expected_moves: Vec<(Pos, Pos)> = vec![];
        assert_eq!(expected_moves, board.add_and_trickle(Pos::new(2, 1), piece2));
    }

    #[test]
    fn add_trickle_with_diagonals_piece_replace_wall_sets_board() {
        let type1 = 'f';
        let piece1 = Piece::Regular(type1, ALL_DIRECTIONS);
        let piece2 = Piece::Regular(type1, enum_set!(Direction::South));

        let mut board = Board::new(BoardState::new(16, 16),
                                   Vec::new(), Vec::new());

        board.set_piece(Pos::new(1, 0), piece1);
        board.set_piece(Pos::new(1, 1), piece1);
        board.set_piece(Pos::new(1, 2), piece1);
        board.set_piece(Pos::new(1, 3), Piece::Empty);
        board.set_piece(Pos::new(1, 4), Piece::Empty);

        board.set_piece(Pos::new(2, 0), piece1);

        board.set_piece(Pos::new(2, 2), Piece::Empty);
        board.set_piece(Pos::new(2, 3), Piece::Empty);
        board.set_piece(Pos::new(2, 4), Piece::Empty);

        board.set_piece(Pos::new(3, 0), Piece::Empty);
        board.set_piece(Pos::new(3, 1), Piece::Empty);
        board.set_piece(Pos::new(3, 2), Piece::Empty);
        board.set_piece(Pos::new(3, 3), Piece::Empty);
        board.set_piece(Pos::new(3, 4), Piece::Empty);

        board.add_and_trickle(Pos::new(2, 1), piece2);

        assert_eq!(piece1, board.piece(Pos::new(1, 0)));
        assert_eq!(piece1, board.piece(Pos::new(1, 1)));
        assert_eq!(piece1, board.piece(Pos::new(1, 2)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(1, 3)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(1, 4)));

        assert_eq!(piece1, board.piece(Pos::new(2, 0)));
        assert_eq!(piece2, board.piece(Pos::new(2, 1)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(2, 2)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(2, 3)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(2, 4)));

        assert_eq!(Piece::Empty, board.piece(Pos::new(3, 0)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(3, 1)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(3, 2)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(3, 3)));
        assert_eq!(Piece::Empty, board.piece(Pos::new(3, 4)));
    }

    #[test]
    fn add_trickle_with_diagonals_piece_replace_wall_generates_moves() {
        let type1 = 'f';
        let piece1 = Piece::Regular(type1, ALL_DIRECTIONS);
        let piece2 = Piece::Regular(type1, enum_set!(Direction::South));

        let mut board = Board::new(BoardState::new(16, 16),
                                   Vec::new(), Vec::new());

        board.set_piece(Pos::new(1, 0), piece1);
        board.set_piece(Pos::new(1, 1), piece1);
        board.set_piece(Pos::new(1, 2), piece1);
        board.set_piece(Pos::new(1, 3), Piece::Empty);
        board.set_piece(Pos::new(1, 4), Piece::Empty);

        board.set_piece(Pos::new(2, 0), piece1);

        board.set_piece(Pos::new(2, 2), Piece::Empty);
        board.set_piece(Pos::new(2, 3), Piece::Empty);
        board.set_piece(Pos::new(2, 4), Piece::Empty);

        board.set_piece(Pos::new(3, 0), Piece::Empty);
        board.set_piece(Pos::new(3, 1), Piece::Empty);
        board.set_piece(Pos::new(3, 2), Piece::Empty);
        board.set_piece(Pos::new(3, 3), Piece::Empty);
        board.set_piece(Pos::new(3, 4), Piece::Empty);

        let expected_moves: Vec<(Pos, Pos)> = vec![];
        assert_eq!(expected_moves, board.add_and_trickle(Pos::new(2, 1), piece2));
    }

    #[test]
    fn display_shows_all_pieces_with_type() {
        let piece1 = Piece::Regular('f', ALL_DIRECTIONS);
        let piece2 = Piece::Regular('s', ALL_DIRECTIONS);

        let mut board = Board::new(BoardState::new(15, 17),
                                   Vec::new(), Vec::new());

        board.set_piece(Pos::new(1, 0), piece1);

        board.set_piece(Pos::new(1, 1), piece2);
        board.set_piece(Pos::new(1, 2), piece1);
        board.set_piece(Pos::new(1, 3), Piece::Empty);
        board.set_piece(Pos::new(1, 4), Piece::Empty);

        board.set_piece(Pos::new(2, 0), piece2);
        board.set_piece(Pos::new(2, 1), piece2);
        board.set_piece(Pos::new(2, 2), Piece::Empty);
        board.set_piece(Pos::new(2, 3), Piece::Empty);
        board.set_piece(Pos::new(2, 4), Piece::Empty);

        board.set_piece(Pos::new(3, 0), piece2);
        board.set_piece(Pos::new(3, 1), piece1);
        board.set_piece(Pos::new(3, 2), piece1);
        board.set_piece(Pos::new(3, 3), Piece::Empty);
        board.set_piece(Pos::new(3, 4), piece2);

        let expected = "\
        ###############\
        \n###############\
        \n###############\
        \n###############\
        \n###############\
        \n###############\
        \n###############\
        \n###############\
        \n###############\
        \n###############\
        \n###############\
        \n###############\
        \n#  s###########\
        \n#   ###########\
        \n#f f###########\
        \n#ssf###########\
        \n#fss###########\
        \n";

        assert_eq!(expected, format!("{}", board));
    }
}