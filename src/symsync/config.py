import collections.abc as c
from pathlib import Path

import msgspec


class SyncEntry(msgspec.Struct, omit_defaults=True):
  "configuration for a single sync operation"

  sources: c.Sequence[str]
  "globs used to find source files for the sync"

  target: str
  "target directory for the sync"

  groups: set[str] = set()
  "a set of group names this sync group belongs to"


class SyncConfig(msgspec.Struct, omit_defaults=True):
  "A mapping of sync entries, keyed by a unique name."

  entries: c.Sequence[SyncEntry]
  "a sequence of sync entries"

  @classmethod
  def parse_file(cls, path: Path):
    """
    Parse the config file at the given path.

    Raises:
      - ValueError: if the file extension is not supported
      - msgspec.DecodeError: if the file contents are not valid JSON, TOML, or YAML
      - FileNotFoundError: if the file is not found
    """

    if not path.exists():
      raise FileNotFoundError(f"Config file not found: [underline]{path}[/]")
    if not path.is_file():
      raise ValueError(f"Not a file: [underline]{path}[/]")

    match path.suffix[1:]:
      case "json":
        try:
          return msgspec.json.decode(path.read_bytes(), type=cls)
        except msgspec.DecodeError as e:
          raise msgspec.DecodeError(f"Invalid JSON in {path}: {e}") from e
      case "toml":
        try:
          return msgspec.toml.decode(path.read_bytes(), type=cls)
        except msgspec.DecodeError as e:
          raise msgspec.DecodeError(f"Invalid TOML in {path}: {e}") from e
      case "yaml" | "yml":
        try:
          return msgspec.yaml.decode(path.read_bytes(), type=cls)
        except msgspec.DecodeError as e:
          raise msgspec.DecodeError(f"Invalid YAML in {path}: {e}") from e
      case suffix:
        raise ValueError(f"Unsupported extension: {suffix}")
