import functools
import inspect
import logging
import time
import typing
from collections.abc import Awaitable, Callable, Generator, Iterable


def wraps[**WrappedParams, WrappedReturn](
  wrapped: Callable[WrappedParams, WrappedReturn],
  assigned: Iterable[str] = functools.WRAPPER_ASSIGNMENTS,
  updated: Iterable[str] = functools.WRAPPER_UPDATES,
):
  """
  Decorator factory to apply update_wrapper() to a wrapper function

  Reimplements `functools.wraps()` with better type annotations.

  Returns a decorator that invokes update_wrapper() with the decorated
  function as the wrapper argument and the arguments to wraps() as the
  remaining arguments. Default arguments are as for update_wrapper().
  This is a convenience function to simplify applying partial() to
  update_wrapper().
  """

  def decorator[**WrapperParams, WrapperReturn](wrapper: Callable[WrapperParams, WrapperReturn]):
    for attr in assigned:
      try:
        value = getattr(wrapped, attr)
      except AttributeError:
        pass
      else:
        setattr(wrapper, attr, value)
    for attr in updated:
      getattr(wrapper, attr).update(getattr(wrapped, attr, {}))

    setattr(wrapper, "__wrapped__", wrapped)  # noqa: B010

    return wrapper

  return decorator


def decorator[**DecoratedParams, DecoratedReturn, **Params, Return](
  decorator_impl: Callable[
    typing.Concatenate[Callable[DecoratedParams, DecoratedReturn], Params], Return
  ],
):
  """
  Meta-decorator factory that simplifies writing decorators with arguments.

  Transforms a function of signature `(fn, *args, **kwargs) -> wrapped_fn`
  into a decorator factory of signature `(*args, **kwargs) -> decorator`.

  ### Usage:
    ```py
    @decorator
    def map[**P, R, T](fn: Callable[P, R], mapper: Callable[[R], T]) -> Callable[P, T]:
      def wrapper(*args: P.args, **kwargs: P.kwargs) -> T:
        return mapper(fn(*args, **kwargs))
      return wrapper
    ```
  """

  @wraps(decorator_impl)
  def decorator_factory(
    *args: Params.args, **kwargs: Params.kwargs
  ) -> Callable[[Callable[DecoratedParams, DecoratedReturn]], Return]:
    def wrapper(fn: Callable[DecoratedParams, DecoratedReturn]) -> Return:
      return decorator_impl(fn, *args, **kwargs)

    return wrapper

  return decorator_factory


@decorator
def map[**P, R, T](fn: Callable[P, R], mapper: Callable[[R], T]):
  """
  Decorator factory that allows one to map the result of a function into another type using a provided mapping function.

  ### Args:
    - `mapper`: A callable that takes the output of the decorated function `R` and transforms into `T`.

  ### Examples:
    Transform the output of a generator into a list:

    ```py
    @decorators.map(list)
    def foo():
      yield 1
      yield 2
      yield 3
    ```
  """

  @wraps(fn)
  def wrapper(*args: P.args, **kwargs: P.kwargs) -> T:
    return mapper(fn(*args, **kwargs))

  return wrapper


@decorator
def map_async[**P, R, T](
  fn: Callable[P, R | Awaitable[R]],
  mapper: Callable[[R], T | Awaitable[T]],
):
  """
  Decorator factory that allows one to map the result of an async function into another type using a provided mapping function.
  This does not work with async generators, only regular async functions that return awaitables.

  ### Args:
    - `mapper`: A callable that takes the output of the decorated function `R` and transforms into `T`.

  ### Examples:
    ```py
    @decorators.map_async(lambda a : a + 1)
    async def foo():
      return await get_value(1)
    ```
  """

  @wraps(fn)
  async def wrapper(*args: P.args, **kwargs: P.kwargs) -> T:
    fn_result = fn(*args, **kwargs)
    if inspect.isawaitable(fn_result):
      fn_result = await typing.cast(Awaitable[R], fn_result)
    else:
      fn_result = typing.cast(R, fn_result)

    mapped = mapper(fn_result)
    if inspect.isawaitable(mapped):
      return typing.cast(T, await mapped)
    else:
      return typing.cast(T, mapped)

  return wrapper


@decorator
def time_execution[**P, R](
  fn: typing.Callable[P, R],
  *,
  logger_name: str | None = None,
  log_level: int = logging.INFO,
  msg_formatter: Callable[[str, float], str] = (
    lambda name, time: f"{name} executed in {time:.6f}s"
  ),
  timer: Callable[[], float] = time.time,
):
  """
  Decorator factory for timing function execution and logging the results.

  This decorator measures how long a decorated function takes to execute and logs
  the execution time using Python's logging module.

  ### Args:
    - `logger_name`: The name of the logger to use.
          - Default: the root logger.
    - `log_level`: The logging level for the timing message
    - `msg_formatter`: A callable that formats the log message. Takes the function name
      and execution time (in seconds) as arguments and returns a formatted string.
    - `timer`: A callable that returns the current time as a `float`.

  ### Returns:
    A decorator function that can be applied to any callable to time its execution.

  ### Examples:
    ```py
    @time_execution()
    def slow_function():
      time.sleep(1)
      return "done"


    # With custom parameters
    @time_execution(logger_name="my_app", log_level=logging.DEBUG)
    def another_function():
      return "result"
    ```
  """
  logger = logging.getLogger(logger_name)

  @functools.wraps(fn)
  def wrapper(*args: P.args, **kwargs: P.kwargs) -> R:
    start = timer()
    result = fn(*args, **kwargs)
    end = timer()

    logger.log(log_level, msg_formatter(fn.__name__, end - start))

    return result

  return wrapper


@decorator
def raise_exception_group[E: Exception, R, **P, S = None](
  fn: typing.Callable[P, Generator[E, None, R | S]],
  message: str,
  stop_sentinel: S = None,
):
  """
  Decorator factory that collects yielded errors from a generator function and raises them as an ExceptionGroup.

  ### Args:
    - `message`: The message to use for the ExceptionGroup.
    - `stop_sentinel`: A special value that, if returned by the generator, indicates that the function returned early.
      This must a value that can be compared with `is`, like `...` or `None`.
      If the generator returns this value, the decorator will raise an ExceptionGroup of any errors it has collected so far (including 0), instead of returning the value.

  ### Examples:
    ```py
    @decorators.raise_exception_group("Failed to parse name")
    def parse_name(name: str):
      if not name:
        yield ValueError("Name cannot be empty")
        return # indicate early return due to errors (we dont need to continue with further checks if name is empty)

      if len(name) > 256:
        yield ValueError("Name cannot be longer than 256 characters")

      if any(c in name for c in '"\\*\\?\\<\\>\\/\\|\\\\:'):
        yield ValueError('Name cannot contain any of the following characters: " \\ * ? < > / | \\ :')

      return Name(name)
    ```
  """

  @wraps(fn)
  def wrapper(*args: P.args, **kwargs: P.kwargs) -> R:
    gen = fn(*args, **kwargs)
    errors: list[E] = []
    try:
      while True:
        errors.append(next(gen))
    except StopIteration as e:
      if errors or e.value is stop_sentinel:
        raise ExceptionGroup(message, errors) from None
      return e.value

  return wrapper


__all__ = ["decorator", "map", "map_async", "raise_exception_group", "time_execution"]
