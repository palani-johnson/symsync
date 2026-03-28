import itertools
from pathlib import Path

import cyclopts
import msgspec
import rich.box
import rich.console
import rich.panel
import rich.traceback

from .config import SyncConfig
from .plan import StateItem, plan_link

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
  help_format="rich",
  console=console,
  error_console=error_console,
  error_formatter=error_formatter,
)


@app.default
def default():
  config_path = get_config_path()
  sync_config = get_config(config_path)

  state_items = list(StateItem.generate(config_path.parent, sync_config))
  state_hashes = {item.state_file_hash: item for item in state_items}

  state_dir = config_path.parent / ".symsync.state"
  state_file = state_dir / "state.json"
  state_store = state_dir / "store"

  if not state_dir.exists():
    state_store.mkdir(parents=True)
    state_file.touch()

  if state_dir.is_dir() and state_file.is_file() and state_store.is_dir():
    # State directory does exist, use it
    ...
  else:
    print_error(
      f"State directory {state_dir} must not exist or must exist and follow the expected structure"
    )
    exit(1)

  for state_item in state_items:
    for plan_step in plan_link(state_item, state_store):
      print(plan_step.message)


def get_config_path():
  paths = [
    Path(f"{name}.{ext}")
    for name, ext in itertools.product(
      ["symsync"],
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

  return path


def get_config(path: Path):
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
def config():
  "Parse symsync config, print in json format and exit"

  config_path = get_config_path()
  sync_config = get_config(config_path)
  console.print_json(msgspec.json.encode(sync_config).decode())


@app.command
def plan():
  "Print the sync plan."

  config_path = get_config_path()
  sync_config = get_config(config_path)

  plan_items = list(StateItem.generate(config_path.parent, sync_config))
  plan_hashes = {item.state_file_hash: item for item in plan_items}
  console.print_json(msgspec.json.encode(plan_items).decode())

  state_dir = config_path.parent / ".symsync.state"
  state_file = state_dir / "state.json"
  state_store = state_dir / "store"

  if not state_dir.exists():
    state_store.mkdir(parents=True)
    state_file.touch()

  if state_dir.is_dir() and state_file.is_file() and state_store.is_dir():
    # State directory does exist, use it
    ...
  else:
    print_error(
      f"State directory {state_dir} must not exist or must exist and follow the expected structure"
    )
    exit(1)


if __name__ == "__main__":
  app()
