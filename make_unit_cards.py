#!/usr/bin/env python3
"""Render 15 board-sized unit hexagons.

Usage:
    python3 make_unit_cards.py [unit_cards.html]
"""

import argparse
import math
from pathlib import Path

PAGE_WIDTH_MM = 215.9
PAGE_HEIGHT_MM = 279.4
PAGE_MARGIN_MM = 5
GAP_MM = 2

BOARD_PAGE_WIDTH_MM = 279.4
BOARD_PAGE_HEIGHT_MM = 215.9
BOARD_PAGE_MARGIN_MM = 5
BOARD_SLOT_HEIGHT_MM = 9
BOARD_SLOT_GAP_MM = 1
BOARD_ROW_COUNTS = [4, 5, 6, 7, 6, 5, 4]

LABELS = ["A", "B", "C", "D", "E", "1", "2", "3", "4", "5", "", "", "", "", ""]


def board_cell_width() -> float:
    content_width = BOARD_PAGE_WIDTH_MM - (2 * BOARD_PAGE_MARGIN_MM)
    content_height = BOARD_PAGE_HEIGHT_MM - (2 * BOARD_PAGE_MARGIN_MM)
    board_height_factor = 2 / math.sqrt(3) + (len(BOARD_ROW_COUNTS) - 1) * math.sqrt(3) / 2
    available_height = content_height - (2 * BOARD_SLOT_HEIGHT_MM) - (2 * BOARD_SLOT_GAP_MM)
    return min(content_width / max(BOARD_ROW_COUNTS), available_height / board_height_factor)


BOARD_CELL_WIDTH_MM = board_cell_width()
CELL_WIDTH_MM = BOARD_CELL_WIDTH_MM * 2 / math.sqrt(3)
CELL_HEIGHT_MM = BOARD_CELL_WIDTH_MM


def cards_per_page() -> tuple[int, int, int]:
    usable_width = PAGE_WIDTH_MM - (2 * PAGE_MARGIN_MM)
    usable_height = PAGE_HEIGHT_MM - (2 * PAGE_MARGIN_MM)
    columns = math.floor((usable_width + GAP_MM) / (CELL_WIDTH_MM + GAP_MM))
    rows = math.floor((usable_height + GAP_MM) / (CELL_HEIGHT_MM + GAP_MM))
    return columns, rows, columns * rows


def hex_points() -> str:
    points = [
        (CELL_WIDTH_MM * 0.25, 0),
        (CELL_WIDTH_MM * 0.75, 0),
        (CELL_WIDTH_MM, CELL_HEIGHT_MM / 2),
        (CELL_WIDTH_MM * 0.75, CELL_HEIGHT_MM),
        (CELL_WIDTH_MM * 0.25, CELL_HEIGHT_MM),
        (0, CELL_HEIGHT_MM / 2),
    ]
    return " ".join(f"{x:.3f},{y:.3f}" for x, y in points)


def arrow_path() -> str:
    x = CELL_WIDTH_MM / 2
    tip_y = CELL_HEIGHT_MM * 0.12
    tail_y = CELL_HEIGHT_MM * 0.35
    wing_y = CELL_HEIGHT_MM * 0.23
    wing_dx = CELL_WIDTH_MM * 0.12
    return (
        f"M {x:.3f} {tail_y:.3f} V {tip_y:.3f} "
        f"M {x - wing_dx:.3f} {wing_y:.3f} L {x:.3f} {tip_y:.3f} "
        f"L {x + wing_dx:.3f} {wing_y:.3f}"
    )


def unit_html(label: str) -> str:
    label_text = ""
    css_class = "blank"
    if label:
        label_text = f'<text class="unit-label" x="{CELL_WIDTH_MM / 2:.3f}" y="{CELL_HEIGHT_MM * 0.58:.3f}">{label}</text>'
        css_class = "letters" if label.isalpha() else "numbers"
    return f"""      <div class=\"unit\">
        <svg viewBox=\"0 0 {CELL_WIDTH_MM:.3f} {CELL_HEIGHT_MM:.3f}\" aria-hidden=\"true\">
          <polygon class=\"cut-line {css_class}\" points=\"{hex_points()}\" />
          <path class=\"arrow\" d=\"{arrow_path()}\" />
          {label_text}
        </svg>
      </div>"""


def page_html(page_size: int, page_class: str = "page") -> str:
    pages = []
    for start in range(0, len(LABELS), page_size):
        page_units = "\n".join(unit_html(label) for label in LABELS[start : start + page_size])
        pages.append(f"""  <section class=\"{page_class}\">
{page_units}
  </section>""")
    return "\n".join(pages)


def render_html() -> str:
    columns, rows, page_size = cards_per_page()
    pages = page_html(page_size)
    label_size = CELL_WIDTH_MM * 0.34
    return f"""<!doctype html>
<html lang=\"en\">
<head>
  <meta charset=\"utf-8\">
  <title>Unit cards</title>
  <style>
    @page {{
      size: Letter portrait;
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
      padding: {PAGE_MARGIN_MM}mm;
      margin: 0 auto;
      background: white;
      display: grid;
      grid-template-columns: repeat({columns}, {CELL_WIDTH_MM:.3f}mm);
      grid-auto-rows: {CELL_HEIGHT_MM:.3f}mm;
      gap: {GAP_MM}mm;
      justify-content: center;
      align-content: center;
      break-after: page;
      page-break-after: always;
    }}

    .page:last-child {{
      break-after: auto;
      page-break-after: auto;
    }}

    .unit {{
      width: {CELL_WIDTH_MM:.3f}mm;
      height: {CELL_HEIGHT_MM:.3f}mm;
    }}

    svg {{
      display: block;
      width: 100%;
      height: 100%;
    }}

    .cut-line {{
      stroke: #111;
      stroke-width: 0.22;
    }}

    .cut-line.blank {{
      fill: white;
    }}

    .cut-line.letters {{
      fill: #fdeaea;
    }}

    .cut-line.numbers {{
      fill: #e9eefa;
    }}

    .arrow {{
      fill: none;
      stroke: #111;
      stroke-width: 0.9;
      stroke-linecap: round;
      stroke-linejoin: round;
    }}

    .unit-label {{
      font-size: {label_size:.3f}px;
      font-weight: 800;
      font-family: Arial, Helvetica, sans-serif;
      text-anchor: middle;
      dominant-baseline: middle;
      fill: #111;
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
{pages}
</body>
</html>
"""


def main() -> None:
    parser = argparse.ArgumentParser(description="Render 15 board-sized unit hexagons on US-letter sheets.")
    parser.add_argument("output", nargs="?", default="unit_cards.html", type=Path)
    args = parser.parse_args()

    args.output.write_text(render_html(), encoding="utf-8")

    columns, rows, page_size = cards_per_page()
    pages = math.ceil(len(LABELS) / page_size)
    print(
        f"wrote {args.output} with {len(LABELS)} units, "
        f"cell width {CELL_WIDTH_MM:.1f}mm, {columns}x{rows} ({page_size}) per page, {pages} page(s)"
    )


if __name__ == "__main__":
    main()
