from os import PathLike

import numpy as np
import numpy.typing as npt

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

class Network:
    def __init__(
        self,
        f: npt.NDArray[np.float64],
        s: npt.NDArray[np.complex128],
        z0: npt.NDArray[np.float64] | None = None,
    ) -> None: ...
    @property
    def f(self) -> npt.NDArray[np.float64]: ...
    @property
    def s(self) -> npt.NDArray[np.complex128]: ...
    @property
    def z0(self) -> npt.NDArray[np.float64]: ...
    @property
    def nports(self) -> int: ...
    @property
    def noise(self) -> NoiseData | None: ...

def read(path: str | PathLike[str]) -> Network: ...
