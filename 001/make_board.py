#!/usr/bin/env python3
"""Render the rules.md board on one US-letter page with power tabs.

Usage:
    python3 make_board.py [rules.md] [board.html]
"""

import argparse
import html
import math
import re
from pathlib import Path

PAGE_WIDTH_MM = 279.4
PAGE_HEIGHT_MM = 215.9
PAGE_MARGIN_MM = 5
CARD_WIDTH_MM = 63.5
SLOT_HEIGHT_MM = 9
SLOT_GAP_MM = 1
SLOT_INNER_GAP_MM = 2

CELL_RE = re.compile(r"[xHF]")


def parse_board(path: Path) -> list[list[str]]:
    lines = path.read_text(encoding="utf-8").splitlines()
    for index, line in enumerate(lines):
        if "Looks like:" in line:
            start = index + 1
            break
    else:
        raise ValueError(f"{path}: could not find board diagram")

    rows: list[list[str]] = []
    for line in lines[start:]:
        if not line.strip():
            if rows:
                break
            continue
        cells = CELL_RE.findall(line)
        if cells:
            rows.append(cells)
        elif rows:
            break

    if not rows:
        raise ValueError(f"{path}: board diagram is empty")
    return rows


def content_size() -> tuple[float, float]:
    return PAGE_WIDTH_MM - (2 * PAGE_MARGIN_MM), PAGE_HEIGHT_MM - (2 * PAGE_MARGIN_MM)


def cell_width_for(rows: list[list[str]]) -> float:
    content_width, content_height = content_size()
    max_columns = max(len(row) for row in rows)
    board_height_factor = 2 / math.sqrt(3) + (len(rows) - 1) * math.sqrt(3) / 2
    available_height = content_height - (2 * SLOT_HEIGHT_MM) - (2 * SLOT_GAP_MM)
    return min(content_width / max_columns, available_height / board_height_factor)


def board_size(rows: list[list[str]], cell_width: float) -> tuple[float, float]:
    max_columns = max(len(row) for row in rows)
    cell_height = cell_width * 2 / math.sqrt(3)
    row_step = cell_height * 0.75
    return max_columns * cell_width, cell_height + (len(rows) - 1) * row_step


def hex_points(cx: float, cy: float, cell_width: float) -> str:
    cell_height = cell_width * 2 / math.sqrt(3)
    half_width = cell_width / 2
    half_height = cell_height / 2
    quarter_height = cell_height / 4
    points = [
        (cx, cy - half_height),
        (cx + half_width, cy - quarter_height),
        (cx + half_width, cy + quarter_height),
        (cx, cy + half_height),
        (cx - half_width, cy + quarter_height),
        (cx - half_width, cy - quarter_height),
    ]
    return " ".join(f"{x:.3f},{y:.3f}" for x, y in points)


def slot_widths(content_width: float) -> list[float]:
    outer_width = (content_width - (3 * CARD_WIDTH_MM) - (4 * SLOT_INNER_GAP_MM)) / 2
    return [outer_width, CARD_WIDTH_MM, CARD_WIDTH_MM, CARD_WIDTH_MM, outer_width]


def slot_row(labels: list[str], y: float, css_class: str, content_width: float) -> str:
    x = 0.0
    pieces = []
    for label, width in zip(labels, slot_widths(content_width)):
        pieces.append(
            f'<rect class="slot {css_class}" x="{x:.3f}" y="{y:.3f}" '
            f'width="{width:.3f}" height="{SLOT_HEIGHT_MM:.3f}" rx="1" />'
        )
        pieces.append(
            f'<text class="slot-label {css_class}" x="{x + width / 2:.3f}" '
            f'y="{y + SLOT_HEIGHT_MM / 2:.3f}">{html.escape(label)}</text>'
        )
        x += width + SLOT_INNER_GAP_MM
    return "\n      ".join(pieces)


def board_cells(rows: list[list[str]], cell_width: float, board_x: float, board_y: float) -> str:
    max_columns = max(len(row) for row in rows)
    cell_height = cell_width * 2 / math.sqrt(3)
    row_step = cell_height * 0.75
    pieces = []
    for row_index, row in enumerate(rows):
        y = board_y + (cell_height / 2) + (row_index * row_step)
        x = board_x + ((max_columns - len(row)) * cell_width / 2) + (cell_width / 2)
        for kind in row:
            css_class = {"x": "empty", "H": "home", "F": "flag"}[kind]
            label = "" if kind == "x" else kind
            text = ""
            if label:
                text = f'<text class="cell-label" x="{x:.3f}" y="{y:.3f}">{html.escape(label)}</text>'
            pieces.append(
                f'<g class="cell {css_class}">'
                f'<polygon points="{hex_points(x, y, cell_width)}" />'
                f'{text}'
                f'</g>'
            )
            x += cell_width
    return "\n      ".join(pieces)


def render_html(rows: list[list[str]]) -> str:
    content_width, content_height = content_size()
    cell_width = cell_width_for(rows)
    board_width, board_height = board_size(rows, cell_width)
    board_x = (content_width - board_width) / 2
    board_area_height = content_height - (2 * SLOT_HEIGHT_MM) - (2 * SLOT_GAP_MM)
    board_y = SLOT_HEIGHT_MM + SLOT_GAP_MM + ((board_area_height - board_height) / 2)
    bottom_slot_y = content_height - SLOT_HEIGHT_MM

    top_slots = slot_row(["A", "B", "C", "D", "E"], 0, "letters", content_width)
    bottom_slots = slot_row(["1", "2", "3", "4", "5"], bottom_slot_y, "numbers", content_width)
    cells = board_cells(rows, cell_width, board_x, board_y)
    cell_label_size = cell_width * 0.32

    return f"""<!doctype html>
<html lang=\"en\">
<head>
  <meta charset=\"utf-8\">
  <title>Board</title>
  <style>
    @page {{
      size: Letter landscape;
      margin: 0;
    }}

    * {{
      box-sizing: border-box;
    }}

    body {{
      margin: 0;
      background: #eee;
      font-family: Arial, Helvetica, sans-serif;
      color: #111;
    }}

    .page {{
      width: {PAGE_WIDTH_MM}mm;
      height: {PAGE_HEIGHT_MM}mm;
      margin: 0 auto;
      padding: {PAGE_MARGIN_MM}mm;
      background: white;
    }}

    svg {{
      display: block;
      width: {content_width:.3f}mm;
      height: {content_height:.3f}mm;
    }}

    .cell polygon {{
      stroke: #111;
      stroke-width: 0.22;
    }}

    .empty polygon {{
      fill: white;
    }}

    .home polygon {{
      fill: #dcecff;
    }}

    .flag polygon {{
      fill: #fff0a8;
    }}

    text {{
      font-family: Arial, Helvetica, sans-serif;
      text-anchor: middle;
      dominant-baseline: middle;
      pointer-events: none;
    }}

    .cell-label {{
      font-size: {cell_label_size:.3f}px;
      font-weight: 800;
      fill: #111;
    }}

    .slot {{
      stroke: #555;
      stroke-width: 0.25;
      stroke-dasharray: 1.6 1.2;
    }}

    .slot.letters {{
      fill: #fdeaea;
    }}

    .slot.numbers {{
      fill: #e9eefa;
    }}

    .slot-label {{
      font-size: 5px;
      font-weight: 800;
      letter-spacing: 1px;
    }}

    .slot-label.letters {{
      fill: #8b2222;
    }}

    .slot-label.numbers {{
      fill: #1f3e7a;
    }}

    @media screen {{
      .page {{
        margin: 8mm auto;
        box-shadow: 0 0 8mm rgba(0, 0, 0, 0.25);
      }}
    }}

    @media print {{
      body {{
        background: white;
      }}
    }}
  </style>
</head>
<body>
  <section class=\"page\">
    <svg viewBox=\"0 0 {content_width:.3f} {content_height:.3f}\" aria-label=\"Game board\">
      {top_slots}
      {cells}
      {bottom_slots}
    </svg>
  </section>
</body>
</html>
"""


def main() -> None:
    parser = argparse.ArgumentParser(description="Render rules.md board as a single US-letter sheet.")
    parser.add_argument("input", nargs="?", default="rules.md", type=Path)
    parser.add_argument("output", nargs="?", default="board.html", type=Path)
    args = parser.parse_args()

    rows = parse_board(args.input)
    args.output.write_text(render_html(rows), encoding="utf-8")

    cell_width = cell_width_for(rows)
    board_width, board_height = board_size(rows, cell_width)
    print(
        f"wrote {args.output} with {sum(len(row) for row in rows)} cells, "
        f"cell width {cell_width:.1f}mm, board {board_width:.1f}mm x {board_height:.1f}mm"
    )


if __name__ == "__main__":
    main()
