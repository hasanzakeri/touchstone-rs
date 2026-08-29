from collections.abc import Sequence
from os import PathLike
from typing import Any, TypeAlias

import numpy as np
import numpy.typing as npt

# Anything the z0 parameter accepts: a numeric array of any dtype, or a
# sequence of numbers, in either accepted shape. Real input is widened -- the
# format's own reference impedances are real, so a caller holding real values
# is the ordinary case and should not have to convert.
Z0Like: TypeAlias = (
    npt.NDArray[np.number[Any]] | Sequence[complex] | Sequence[Sequence[complex]]
)

__version__: str

class TouchstoneError(ValueError):
    """Raised when a Touchstone file cannot be read or parsed."""

class NoiseData:
    @property
    def f(self) -> npt.NDArray[np.float64]: ...
    @property
    def nfmin_db(self) -> npt.NDArray[np.float64]: ...
    @property
    def gamma_opt(self) -> npt.NDArray[np.complex128]: ...
    @property
    def rn(self) -> npt.NDArray[np.float64]: ...
    @property
    def rn_ohms(self) -> npt.NDArray[np.float64]: ...

class Network:
    def __init__(
        self,
        f: npt.NDArray[np.float64],
        s: npt.NDArray[np.complex128],
        # Shaped (N,) -- one per port, tiled over the sweep -- or (F, N).
        z0: Z0Like | None = None,
    ) -> None: ...
    @property
    def f(self) -> npt.NDArray[np.float64]: ...
    @property
    def s(self) -> npt.NDArray[np.complex128]: ...
    @property
    def z0(self) -> npt.NDArray[np.complex128]: ...
    @property
    def nports(self) -> int: ...
    @property
    def noise(self) -> NoiseData | None: ...

def read(path: str | PathLike[str]) -> Network: ...
