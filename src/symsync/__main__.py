import itertools
from pathlib import Path

import cyclopts
import cyclopts.help
import msgspec
import rich.box
import rich.console
import rich.panel
import rich.traceback

from .config import SyncConfig

# Rich console setup
console = rich.console.Console(tab_size=2)
error_console = rich.console.Console(stderr=True, tab_size=2)
rich.traceback.install(console=error_console)


def error_formatter(error: object):
  return rich.panel.Panel(
    str(error),
    title="Error",
    border_style="red",
    box=rich.box.ROUNDED,
    expand=False,
    title_align="left",
  )


def print_error(error: object):
  error_console.print(error_formatter(error))


# Cyclopts app setup
app = cyclopts.App(
  help_format="md",
  console=console,
  error_console=error_console,
  error_formatter=error_formatter,
)


@app.default
def default(path: Path | None = None):
  sync_config = get_config(path)
  console.print(sync_config)


def get_config(path: Path | None):
  if path is None:
    paths = [
      Path(f"{name}.{ext}")
      for name, ext in itertools.product(
        ["symsync", ".symsync"],
        ["toml", "json", "yaml", "yml"],
      )
    ]

    path = next((p for p in paths if p.is_file()), None)

    if path is None:
      print_error(
        "No config file found. Tried the following paths:\n"
        + "\n".join(f"\t- [underline]{p}[/]" for p in paths),
      )
      exit(1)

  try:
    return SyncConfig.parse_file(path)
  except msgspec.DecodeError as e:
    print_error(e)
    exit(1)
  except FileNotFoundError as e:
    print_error(e)
    exit(1)
  except ValueError as e:
    print_error(e)
    exit(1)


@app.command
def config(path: Path | None = None):
  config = get_config(path)
  console.print_json(msgspec.json.encode(config).decode())


if __name__ == "__main__":
  app()
