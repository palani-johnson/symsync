import collections.abc as c
import hashlib
import typing as t
from pathlib import Path

import msgspec

from .config import SyncConfig


class StateItem(msgspec.Struct, omit_defaults=True):
  "configuration for a single state item"

  link_path: str
  "the path that the link will be created at"

  link: str
  "what the symlink will point to"

  state_file_hash: str
  "unique key into the state dir"

  groups: set[str] = set()
  "a set of group names this plan item belongs to"

  @classmethod
  def generate(cls, root_dir: Path, sync_config: SyncConfig):
    for entry in sync_config.entries:
      target = Path(entry.target).absolute()

      for source in entry.sources:
        for file in root_dir.glob(source):
          if file.is_file():
            file = file.relative_to(root_dir)
            state_file_hash = hashlib.sha1(
              str(file).encode(),
              usedforsecurity=False,
            ).hexdigest()

            yield cls(
              link_path=str(target),
              link=str(file),
              state_file_hash=state_file_hash,
              groups=entry.groups,
            )


class PlanStep(t.Protocol):
  "a single planned step to execute"

  message: str

  def execute(self) -> None: ...


class NoOpStep(msgspec.Struct):
  "A no-op plan item."

  message: str

  def execute(self):
    pass


class ErrorStep(msgspec.Struct):
  "A plan item that raises an error"

  message: str

  def execute(self):
    raise RuntimeError(self.message)


class SymlinkStep(msgspec.Struct):
  "A plan item that creates a symlink"

  message: str
  symlink_path: str
  target_path: str

  def execute(self):
    Path(self.symlink_path).symlink_to(self.target_path)


class MkdirStep(msgspec.Struct):
  "A plan item that creates a directory"

  message: str
  path: str

  def execute(self):
    Path(self.path).mkdir(parents=True)


def plan_link(state_item: StateItem, state_store: Path) -> c.Iterator[PlanStep]:
  "Yields a plan for a single state item, given a state store path and a console."

  # extract the three paths we need
  link_path = Path(state_item.link_path) / state_item.link
  true_path = Path(state_item.link).absolute()
  store_path = (state_store / state_item.state_file_hash).absolute()
  store_to_true_path = true_path.relative_to(store_path, walk_up=True)

  # try to make a link from the store to the true path
  if store_path.is_symlink():
    if store_path.readlink() == store_to_true_path:
      yield NoOpStep(f"Store path {store_path} already exists")
    else:
      yield ErrorStep(f"Store path {store_path} exists but points to an invalid location")
  elif store_path.exists(follow_symlinks=False):
    yield ErrorStep(f"Store path {store_path} exists but is not a symlink")
  else:
    yield SymlinkStep(
      f"Store Path {store_path} symlinked to {store_to_true_path}",
      symlink_path=str(store_path),
      target_path=str(store_to_true_path),
    )

  # try to make the link_path parent directory
  if link_path.parent.exists():
    if link_path.parent.is_dir():
      yield NoOpStep(f"Link parent directory {link_path.parent} already exists")
    else:
      yield ErrorStep(f"Link parent directory {link_path.parent} exists but is not a dir")
  else:
    yield MkdirStep(
      f"Link parent directory {link_path.parent} created",
      path=str(link_path.parent),
    )

  # try to make a link from the link_path to the true_path
  if link_path.is_symlink() and link_path.readlink() == store_path:
    yield NoOpStep(f"Link path {link_path} already exists")
  elif link_path.exists(follow_symlinks=False):
    yield ErrorStep(f"Link path {link_path} exists but is not a symlink")
  else:
    yield SymlinkStep(
      f"Link path {link_path} symlinked to {store_path}",
      symlink_path=str(link_path),
      target_path=str(store_path),
    )
