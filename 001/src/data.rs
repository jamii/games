//! Static game data: the board (parsed from the diagram in `rules.md`) and the
//! power cards (parsed from `powers.md`). Both files are embedded at compile
//! time, so editing them and rebuilding keeps the app in sync with the rules.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::OnceLock;

pub type Seat = usize;

pub const RULES_MD: &str = include_str!("../../rules.md");
pub const POWERS_MD: &str = include_str!("../../powers.md");

/// A board space in "doubled width" coordinates: `y` is the row, and `x`
/// steps by 2 between horizontally adjacent spaces.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Cell {
    pub x: i32,
    pub y: i32,
}

/// Neighbour offsets, clockwise (on screen) starting from east:
/// E, SE, SW, W, NW, NE.
pub const DIRS: [(i32, i32); 6] = [(2, 0), (1, 1), (-1, 1), (-2, 0), (-1, -1), (1, -1)];

impl Cell {
    pub fn step(self, dir: usize) -> Cell {
        let (dx, dy) = DIRS[dir % 6];
        Cell {
            x: self.x + dx,
            y: self.y + dy,
        }
    }

    pub fn neighbors(self) -> impl Iterator<Item = Cell> {
        (0..6).map(move |dir| self.step(dir))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CellKind {
    Empty,
    Home(Seat),
    Flag,
}

#[derive(Debug)]
pub struct Board {
    pub cells: Vec<(Cell, CellKind)>,
    kinds: HashMap<Cell, CellKind>,
    pub flag: Cell,
}

impl Board {
    /// Parses the diagram that follows "Looks like:" in rules.md. Rows are
    /// centred, so neighbouring rows must differ in length by an odd number.
    /// Home spaces in the top half belong to seat 0, the bottom half to seat 1.
    pub fn parse(md: &str) -> Result<Board, String> {
        let lines: Vec<&str> = md.lines().collect();
        let start = lines
            .iter()
            .position(|line| line.contains("Looks like:"))
            .ok_or("could not find the board diagram (a line containing 'Looks like:')")?
            + 1;

        let mut rows: Vec<Vec<char>> = Vec::new();
        for line in &lines[start..] {
            let row: Vec<char> = line
                .chars()
                .filter(|c| matches!(c, 'x' | 'H' | 'F'))
                .collect();
            if line.trim().is_empty() || row.is_empty() {
                if rows.is_empty() {
                    continue;
                }
                break;
            }
            rows.push(row);
        }
        if rows.is_empty() {
            return Err("the board diagram is empty".into());
        }
        for (i, pair) in rows.windows(2).enumerate() {
            if (pair[0].len() as i32 - pair[1].len() as i32).abs() % 2 != 1 {
                return Err(format!(
                    "board rows {} and {} must differ in length by an odd number of spaces",
                    i + 1,
                    i + 2
                ));
            }
        }

        let row_count = rows.len() as i32;
        let mut cells = Vec::new();
        let mut kinds = HashMap::new();
        let mut flag = None;
        for (y, row) in rows.iter().enumerate() {
            let y = y as i32;
            let len = row.len() as i32;
            for (col, ch) in row.iter().enumerate() {
                let cell = Cell {
                    x: 2 * col as i32 - (len - 1),
                    y,
                };
                let kind = match ch {
                    'H' if 2 * y < row_count - 1 => CellKind::Home(0),
                    'H' if 2 * y > row_count - 1 => CellKind::Home(1),
                    'H' => return Err("home spaces can't be on the middle row".into()),
                    'F' => {
                        if flag.replace(cell).is_some() {
                            return Err("the board has more than one flag space".into());
                        }
                        CellKind::Flag
                    }
                    _ => CellKind::Empty,
                };
                cells.push((cell, kind));
                kinds.insert(cell, kind);
            }
        }
        let flag = flag.ok_or("the board has no flag space (F)")?;
        for seat in 0..2 {
            if !cells.iter().any(|(_, kind)| *kind == CellKind::Home(seat)) {
                return Err(format!(
                    "the board has no home spaces for player {}",
                    seat + 1
                ));
            }
        }
        Ok(Board { cells, kinds, flag })
    }

    pub fn kind(&self, cell: Cell) -> Option<CellKind> {
        self.kinds.get(&cell).copied()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    Active,
    Passive,
    Other,
}

#[derive(Clone, Debug, Serialize)]
pub struct Power {
    pub name: String,
    pub kind: Kind,
    pub text: String,
}

/// Parses lines of the form `*Name*. Active. Text` (blank lines are ignored).
pub fn parse_powers(md: &str) -> Result<Vec<Power>, String> {
    let mut powers = Vec::new();
    for (i, raw) in md.lines().enumerate() {
        let line = raw.trim();
        if line.is_empty() {
            continue;
        }
        let err = || format!("line {}: expected '*Name*. Text'", i + 1);
        let rest = line.strip_prefix('*').ok_or_else(err)?;
        let end = rest.find('*').ok_or_else(err)?;
        let name = rest[..end].trim().to_string();
        let text = rest[end + 1..].strip_prefix('.').ok_or_else(err)?.trim();
        let (kind, text) = if let Some(text) = text.strip_prefix("Active.") {
            (Kind::Active, text.trim())
        } else if let Some(text) = text.strip_prefix("Passive.") {
            (Kind::Passive, text.trim())
        } else {
            (Kind::Other, text)
        };
        powers.push(Power {
            name,
            kind,
            text: text.to_string(),
        });
    }
    if powers.is_empty() {
        return Err("no powers found".into());
    }
    Ok(powers)
}

pub struct Data {
    pub board: Board,
    pub powers: Vec<Power>,
    /// The *Flag* power.
    pub flag: usize,
    /// The default free power (*Walk*).
    pub walk: usize,
    /// *Fast* (extra activation) and *Strong* (counts double for scoring),
    /// which the app accounts for automatically.
    pub fast: Option<usize>,
    pub strong: Option<usize>,
}

impl Data {
    fn load() -> Result<Data, String> {
        let board = Board::parse(RULES_MD).map_err(|e| format!("rules.md: {e}"))?;
        let powers = parse_powers(POWERS_MD).map_err(|e| format!("powers.md: {e}"))?;
        let find = |name: &str| {
            powers
                .iter()
                .position(|p| p.name.eq_ignore_ascii_case(name))
        };
        let flag = find("Flag").ok_or("powers.md: there is no *Flag* power")?;
        let walk = find("Walk")
            .or_else(|| powers.iter().position(|p| p.kind == Kind::Active))
            .ok_or("powers.md: there are no active powers")?;
        Ok(Data {
            fast: find("Fast"),
            strong: find("Strong"),
            board,
            powers,
            flag,
            walk,
        })
    }
}

static DATA: OnceLock<Data> = OnceLock::new();

pub fn data() -> &'static Data {
    DATA.get_or_init(|| Data::load().unwrap_or_else(|e| panic!("{e}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn board_matches_rules() {
        let board = &data().board;
        assert_eq!(board.cells.len(), 37);
        assert_eq!(board.flag, Cell { x: 0, y: 3 });
        for seat in 0..2 {
            let homes = board
                .cells
                .iter()
                .filter(|(_, k)| *k == CellKind::Home(seat))
                .count();
            assert_eq!(homes, 3);
        }
        assert_eq!(board.kind(Cell { x: -1, y: 0 }), Some(CellKind::Home(0)));
        assert_eq!(board.kind(Cell { x: 0, y: 1 }), Some(CellKind::Home(0)));
        assert_eq!(board.kind(Cell { x: 0, y: 5 }), Some(CellKind::Home(1)));
        // Every space in the middle of the board has six neighbours.
        let inner = board
            .flag
            .neighbors()
            .filter(|c| board.kind(*c).is_some())
            .count();
        assert_eq!(inner, 6);
    }

    #[test]
    fn powers_parse() {
        let d = data();
        assert!(d.powers.len() > 50);
        assert_eq!(d.powers[d.walk].name, "Walk");
        assert_eq!(d.powers[d.walk].kind, Kind::Active);
        assert_eq!(d.powers[d.flag].kind, Kind::Passive);
        assert!(d.fast.is_some() && d.strong.is_some());
    }
}
