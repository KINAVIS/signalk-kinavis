#!/usr/bin/env python3
"""Writes the replay for the end-to-end check: the recorded sentences with
the comments dropped and own vessel's RMC stamped from now on, five seconds
apart, as a receiver running today would stamp them.

Usage: replay.py <recording> <output>
"""

import datetime
import functools
import sys


def checksum(body: str) -> str:
    return format(functools.reduce(lambda total, char: total ^ ord(char), body, 0), "02X")


def main() -> None:
    source, target = sys.argv[1], sys.argv[2]
    now = datetime.datetime.now(datetime.timezone.utc)
    fixes = 0
    lines = []
    with open(source, encoding="ascii") as recording:
        for line in recording:
            line = line.rstrip("\n")
            if not line or line.startswith("#"):
                continue
            if line.startswith("$GPRMC"):
                fields = line[1 : line.index("*")].split(",")
                at = now + datetime.timedelta(seconds=5 * fixes)
                fields[1] = at.strftime("%H%M%S")
                fields[9] = at.strftime("%d%m%y")
                body = ",".join(fields)
                line = f"${body}*{checksum(body)}"
                fixes += 1
            lines.append(line)
    with open(target, "w", encoding="ascii") as out:
        out.write("\n".join(lines) + "\n")


if __name__ == "__main__":
    main()
