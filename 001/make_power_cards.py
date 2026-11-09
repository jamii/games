#!/usr/bin/env python3
"""Render powers.md as printable poker-sized cards.

Usage:
    python3 make_power_cards.py [powers.md] [cards.html]
"""

import argparse
import html
import math
import re
from pathlib import Path

import make_unit_cards

PAGE_WIDTH_MM = 215.9
PAGE_HEIGHT_MM = 279.4
PAGE_MARGIN_MM = 5
CARD_WIDTH_MM = 63.5
CARD_HEIGHT_MM = 88.9
GAP_MM = 1.0

LINE_RE = re.compile(r"^\*(?P<name>[^*]+)\*\.\s*(?P<text>.*)$")
EM_RE = re.compile(r"\*([^*]+)\*")
TYPE_RE = re.compile(r"^(Active|Passive)\.\s*(.*)$")


def parse_cards(path: Path) -> list[dict[str, str]]:
    cards: list[dict[str, str]] = []
    for line_number, raw_line in enumerate(path.read_text(encoding="utf-8").splitlines(), 1):
        line = raw_line.strip()
        if not line:
            continue
        match = LINE_RE.match(line)
        if not match:
            raise ValueError(f"{path}:{line_number}: expected '*Name*. Text'")
        cards.append({"name": match.group("name"), "text": match.group("text")})
    return cards


def inline_markdown(text: str) -> str:
    type_match = TYPE_RE.match(text)
    if type_match:
        kind = html.escape(type_match.group(1))
        rest = html.escape(type_match.group(2))
        rest = EM_RE.sub(r"<em>\1</em>", rest)
        return f"<strong>{kind}</strong><br><span>{rest}</span>"

    escaped = html.escape(text)
    return EM_RE.sub(r"<em>\1</em>", escaped)


def cards_per_page() -> tuple[int, int, int]:
    usable_width = PAGE_WIDTH_MM - (2 * PAGE_MARGIN_MM)
    usable_height = PAGE_HEIGHT_MM - (2 * PAGE_MARGIN_MM)
    columns = math.floor((usable_width + GAP_MM) / (CARD_WIDTH_MM + GAP_MM))
    rows = math.floor((usable_height + GAP_MM) / (CARD_HEIGHT_MM + GAP_MM))
    return columns, rows, columns * rows


def card_html(card: dict[str, str]) -> str:
    name = html.escape(card["name"])
    text = inline_markdown(card["text"])
    return f"""      <div class=\"card\">
        <div class=\"card-name\">{name}</div>
        <div class=\"card-text\"><span>{text}</span></div>
      </div>"""


def page_html(cards: list[dict[str, str]], page_size: int) -> str:
    pages = []
    for start in range(0, len(cards), page_size):
        page_cards = "\n".join(card_html(card) for card in cards[start : start + page_size])
        pages.append(f"""  <section class=\"page\">
{page_cards}
  </section>""")
    return "\n".join(pages)


def mixed_page_html(cards: list[dict[str, str]]) -> str:
    page_cards = "\n".join(card_html(card) for card in cards)
    units = "\n".join(make_unit_cards.unit_html(label) for label in make_unit_cards.LABELS)
    return f"""  <section class=\"page mixed-page\">
    <div class=\"mixed-card-grid\">
{page_cards}
    </div>
    <div class=\"unit-grid\">
{units}
    </div>
  </section>"""


def combined_pages_html(cards: list[dict[str, str]], page_size: int, columns: int) -> tuple[str, int]:
    if not cards:
        unit_columns, _, unit_page_size = make_unit_cards.cards_per_page()
        unit_pages = make_unit_cards.page_html(unit_page_size, page_class="page unit-page")
        return unit_pages, math.ceil(len(make_unit_cards.LABELS) / (unit_columns * make_unit_cards.cards_per_page()[1]))

    final_page_count = len(cards) % page_size or page_size
    if final_page_count <= columns:
        split_at = len(cards) - final_page_count
        full_pages = page_html(cards[:split_at], page_size)
        mixed_page = mixed_page_html(cards[split_at:])
        pages = "\n".join(page for page in [full_pages, mixed_page] if page)
        return pages, math.ceil(split_at / page_size) + 1

    power_pages = page_html(cards, page_size)
    _, _, unit_page_size = make_unit_cards.cards_per_page()
    unit_pages = make_unit_cards.page_html(unit_page_size, page_class="page unit-page")
    pages = "\n".join(page for page in [power_pages, unit_pages] if page)
    return pages, math.ceil(len(cards) / page_size) + math.ceil(len(make_unit_cards.LABELS) / unit_page_size)


def render_html(cards: list[dict[str, str]]) -> str:
    columns, rows, page_size = cards_per_page()
    pages, _ = combined_pages_html(cards, page_size, columns)
    unit_columns, _, _ = make_unit_cards.cards_per_page()
    unit_label_size = make_unit_cards.CELL_WIDTH_MM * 0.34
    return f"""<!doctype html>
<html lang=\"en\">
<head>
  <meta charset=\"utf-8\">
  <title>Power and unit cards</title>
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
      grid-template-columns: repeat({columns}, {CARD_WIDTH_MM}mm);
      grid-auto-rows: {CARD_HEIGHT_MM}mm;
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

    .card {{
      width: {CARD_WIDTH_MM}mm;
      height: {CARD_HEIGHT_MM}mm;
      border: 0.25mm solid #111;
      border-radius: 3mm;
      padding: 6mm 5mm;
      display: flex;
      flex-direction: column;
      background: white;
      overflow: hidden;
    }}

    .card-name {{
      text-align: center;
      font-size: 16pt;
      font-weight: 800;
      line-height: 1;
      padding-bottom: 4mm;
      border-bottom: 0.2mm solid #999;
    }}

    .card-text {{
      flex: 1;
      display: flex;
      align-items: center;
      justify-content: center;
      text-align: center;
      font-size: 10pt;
      line-height: 1.2;
      overflow: hidden;
      hyphens: auto;
    }}

    .card-text strong {{
      font-size: 11pt;
    }}

    .mixed-page {{
      display: flex;
      flex-direction: column;
      align-items: center;
      justify-content: center;
      gap: 12mm;
    }}

    .mixed-card-grid {{
      display: grid;
      grid-template-columns: repeat({columns}, {CARD_WIDTH_MM}mm);
      grid-auto-rows: {CARD_HEIGHT_MM}mm;
      gap: {GAP_MM}mm;
      justify-content: center;
    }}

    .unit-page {{
      grid-template-columns: repeat({unit_columns}, {make_unit_cards.CELL_WIDTH_MM:.3f}mm);
      grid-auto-rows: {make_unit_cards.CELL_HEIGHT_MM:.3f}mm;
      gap: {make_unit_cards.GAP_MM}mm;
    }}

    .unit-grid {{
      display: grid;
      grid-template-columns: repeat(5, {make_unit_cards.CELL_WIDTH_MM:.3f}mm);
      grid-auto-rows: {make_unit_cards.CELL_HEIGHT_MM:.3f}mm;
      gap: {make_unit_cards.GAP_MM}mm;
      justify-content: center;
    }}

    .unit {{
      width: {make_unit_cards.CELL_WIDTH_MM:.3f}mm;
      height: {make_unit_cards.CELL_HEIGHT_MM:.3f}mm;
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
      font-size: {unit_label_size:.3f}px;
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
    parser = argparse.ArgumentParser(description="Render powers.md as printable poker-sized card sheets.")
    parser.add_argument("input", nargs="?", default="powers.md", type=Path)
    parser.add_argument("output", nargs="?", default="cards.html", type=Path)
    args = parser.parse_args()

    cards = parse_cards(args.input)
    html_text = render_html(cards)
    args.output.write_text(html_text, encoding="utf-8")

    columns, rows, page_size = cards_per_page()
    _, pages = combined_pages_html(cards, page_size, columns)
    print(
        f"wrote {args.output} with {len(cards)} cards and {len(make_unit_cards.LABELS)} units, "
        f"{columns}x{rows} ({page_size}) cards per page, {pages} page(s)"
    )


if __name__ == "__main__":
    main()
