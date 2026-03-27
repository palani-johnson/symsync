import collections.abc as c
from pathlib import Path

import msgspec


class GroupEntry(msgspec.Struct):
  sources: list[Path]
  target: Path
  relative: bool = True


type GroupEntries = c.Mapping[str, GroupEntry]
