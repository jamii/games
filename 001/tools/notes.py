#!/usr/bin/env python3
"""The notes players left in the chat box (every chat line is kept as a note),
from the server's journal.

    tools/notes.py [--dir logs] [--days 7]       list them, oldest first
    tools/notes.py --save N FILE                 save note N for replaying:
        cargo run --release --example replay -- FILE

Each note shows when and by whom it was written, the game (its seed) and turn,
what the player's screen showed, and the game's last few log lines. The saved
file holds the game's options and every action up to the note, so replaying
it reaches exactly the position the player was looking at.
"""
import argparse
import glob
import json
import os
import sys


def notes(directory, days):
    files = sorted(glob.glob(os.path.join(directory, 'journal-*.jsonl')))[-days - 1:]
    out = []
    for path in files:
        with open(path) as f:
            for line in f:
                try:
                    record = json.loads(line)
                except json.JSONDecodeError:
                    continue  # a line cut short by a crash
                if record.get('kind') == 'comment':
                    out.append(record)
    return out


def show(i, note):
    seat = {0: 'Letters', 1: 'Numbers'}.get(note.get('seat'), 'a spectator')
    ctx = note.get('context') or {}
    print(f"[{i}] {note.get('time', '?')}  {seat}, game {note.get('game')}, turn {note.get('turn')}")
    print(f"    {note.get('text')}")
    details = []
    if ctx.get('showing'):
        details.append(f"showing \"{ctx['showing']}\"")
    if ctx.get('selected'):
        details.append(f"selected {ctx['selected'].get('name') or ctx['selected']}")
    if ctx.get('picked'):
        details.append(f"picked {ctx['picked']}")
    if ctx.get('screen'):
        details.append(f"screen {ctx['screen']}")
    if details:
        print('    (' + '; '.join(details) + ')')
    for line in (note.get('log') or [])[-6:]:
        print(f"      | {line}")
    print()


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument('--dir', default=os.environ.get('LOG_DIR', 'logs'))
    ap.add_argument('--days', type=int, default=7)
    ap.add_argument('--save', nargs=2, metavar=('N', 'FILE'))
    args = ap.parse_args()
    found = notes(args.dir, args.days)
    if args.save:
        n, path = int(args.save[0]), args.save[1]
        if not 0 <= n < len(found):
            sys.exit(f'there are {len(found)} notes')
        with open(path, 'w') as f:
            json.dump(found[n], f)
        print(f'saved note {n}; replay it with: cargo run --release --example replay -- {path}')
        return
    if not found:
        print(f'No notes in {args.dir}/journal-*.jsonl.')
    for i, note in enumerate(found):
        show(i, note)


if __name__ == '__main__':
    main()
