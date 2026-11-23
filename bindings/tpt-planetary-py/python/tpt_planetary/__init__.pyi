"""Type stubs for tpt_planetary. Must match the names exported by the extension."""

from collections.abc import Mapping, Sequence
from os import PathLike
from typing import Final, Literal, TypeAlias, overload

import numpy as np
from numpy.typing import NDArray

__version__: Final[str]
CORE_VERSION: Final[str]

Calendar: TypeAlias = Literal["gregorian", "julian"]
Float64Array: TypeAlias = NDArray[np.float64]
HeaderValue: TypeAlias = str | bool | int | float
Index: TypeAlias = int | slice
Selection: TypeAlias = Index | tuple[Index, ...] | None

class TptPlanetaryError(Exception):
    """Base class for tpt-planetary errors that are neither bad input nor I/O failures."""

class JulianDate:
    """Two-part Julian Date: an integer ``day`` and a ``fraction`` in ``[0, 1)``.

    The scale (UTC, TAI or TT) is not stored; the caller states it.
    """

    def __new__(cls, day: float, frac: float = 0.0) -> JulianDate: ...
    @staticmethod
    def from_calendar(
        year: int,
        month: int,
        day: int,
        seconds_of_day: float = 0.0,
        calendar: Calendar = "gregorian",
    ) -> JulianDate:
        """Build from a calendar date. ``seconds_of_day`` must be in ``[0, 86400)``.

        Raises ValueError for a non-existent date or an out-of-range field.
        """
        ...
    def to_calendar(self, calendar: Calendar = "gregorian") -> tuple[int, int, int, float]:
        """Return ``(year, month, day, seconds_of_day)``."""
        ...
    @property
    def day(self) -> float: ...
    @property
    def fraction(self) -> float: ...
    def to_float(self) -> float:
        """``day + fraction`` as one float64 (loses precision near J2000)."""
        ...
    def to_mjd(self) -> tuple[float, float]:
        """Modified Julian Date as ``(day, fraction)``."""
        ...
    def add_days(self, days: float) -> JulianDate: ...
    def add_seconds(self, seconds: float) -> JulianDate: ...
    def days_since(self, other: JulianDate) -> float: ...
    def seconds_since(self, other: JulianDate) -> float: ...
    def __eq__(self, other: object) -> bool: ...
    def __repr__(self) -> str: ...

def utc_to_tai(jd: JulianDate) -> JulianDate:
    """Convert UTC to TAI using the embedded leap-second table."""
    ...
def tai_to_utc(jd: JulianDate) -> JulianDate:
    """Convert TAI to UTC using the embedded leap-second table."""
    ...
def tai_to_tt(jd: JulianDate) -> JulianDate:
    """Convert TAI to TT (fixed offset of 32.184 s)."""
    ...
def tt_to_tai(jd: JulianDate) -> JulianDate:
    """Convert TT to TAI (fixed offset of 32.184 s)."""
    ...
def utc_to_tt(jd: JulianDate) -> JulianDate:
    """Convert UTC to TT."""
    ...
def tt_to_utc(jd: JulianDate) -> JulianDate:
    """Convert TT to UTC."""
    ...
def leap_second_table_info() -> dict[str, object]:
    """Describe the embedded leap-second table.

    Keys: ``version`` (int), ``source`` (str), ``valid_until`` (year, month, day),
    ``entries`` (int), ``last_tai_minus_utc`` (float seconds).
    """
    ...
def leap_second_table_expired(jd: JulianDate) -> bool:
    """True if the embedded table does not cover the civil day of ``jd``."""
    ...
def angular_separation(
    lon1: Float64Array, lat1: Float64Array, lon2: Float64Array, lat2: Float64Array
) -> Float64Array:
    """Great-circle separation in degrees between paired positions (degrees in, degrees out).

    Arguments are 1-D float64 arrays of equal length. Other dtypes raise TypeError.
    """
    ...
def icrs_to_galactic(ra: Float64Array, dec: Float64Array) -> tuple[Float64Array, Float64Array]:
    """ICRS (RA, Dec) in degrees to Galactic ``(l, b)`` in degrees."""
    ...
def galactic_to_icrs(l: Float64Array, b: Float64Array) -> tuple[Float64Array, Float64Array]:  # noqa: E741
    """Galactic ``(l, b)`` in degrees to ICRS ``(ra, dec)`` in degrees."""
    ...
def earth_rotation_angle(ut1: JulianDate) -> float:
    """Earth Rotation Angle in radians for a UT1 Julian Date."""
    ...
def gmst_iau2006(ut1: JulianDate, tt: JulianDate) -> float:
    """Greenwich Mean Sidereal Time (IAU 2006) in radians."""
    ...

class Wcs:
    """Celestial WCS (TAN, SIN or AIT). Pixel coordinates are FITS 1-based.

    Positions outside the projection's domain are returned as NaN.
    """

    @staticmethod
    def from_header(header: Mapping[str, HeaderValue]) -> Wcs:
        """Build from a header mapping such as ``{"CRPIX1": 1.0, "CTYPE1": "RA---TAN", ...}``.

        Raises ValueError for a missing or unsupported projection, mismatched
        axes, a singular matrix or an invalid keyword.
        """
        ...
    @property
    def crval(self) -> tuple[float, float]: ...
    def pixel_to_world(
        self, x: Float64Array, y: Float64Array
    ) -> tuple[Float64Array, Float64Array]:
        """Pixel ``(x, y)`` to ``(lon, lat)`` in degrees."""
        ...
    def world_to_pixel(
        self, lon: Float64Array, lat: Float64Array
    ) -> tuple[Float64Array, Float64Array]:
        """``(lon, lat)`` in degrees to pixel ``(x, y)``."""
        ...

class ZarrArray:
    """Zarr v2 or v3 array in a local directory. Reads return float64 arrays."""

    @property
    def shape(self) -> tuple[int, ...]: ...
    @property
    def ndim(self) -> int: ...
    @property
    def chunks(self) -> tuple[int, ...]: ...
    @property
    def dtype(self) -> str:
        """Stored dtype in NumPy typestr form, for example ``"<f8"``."""
        ...
    @property
    def path(self) -> str: ...
    @overload
    def read(self, selection: None = None) -> Float64Array: ...
    @overload
    def read(self, selection: Selection) -> Float64Array | float: ...
    def read(self, selection: Selection = None) -> Float64Array | float:
        """Read a selection as a new float64 array (a float if no dimension remains).

        Integers drop a dimension; slices must have a positive step.
        """
        ...
    def sum(self) -> float: ...
    def mean(self) -> float | None: ...
    def min(self) -> float | None: ...
    def max(self) -> float | None: ...

def nino34(field: ZarrArray, lat_deg: Sequence[float], lon_deg: Sequence[float]) -> Float64Array:
    """Niño-3.4 SST anomaly index, one value per month.

    ``field`` has shape ``(time, lat, lon)`` with monthly steps; the coordinates
    are in degrees (latitudes increasing, longitudes east and increasing).
    """
    ...

def nino34_relative(
    field: ZarrArray, lat_deg: Sequence[float], lon_deg: Sequence[float]
) -> Float64Array:
    """Niño-3.4 anomaly minus the 20°S–20°N mean anomaly of the same month."""
    ...

class NetCdf3File:
    """A NetCDF-3 (classic or 64-bit offset) file read into memory.

    Variables are read as float64. NetCDF-4 (HDF5) files are not supported.
    """

    @property
    def variables(self) -> list[str]:
        """Variable names, in file order."""
        ...
    @property
    def dimensions(self) -> dict[str, int]:
        """Dimension names and lengths; the unlimited dimension is its record count."""
        ...
    def shape(self, name: str) -> tuple[int, ...]:
        """Shape of variable ``name``."""
        ...
    def read(self, name: str, selection: Selection = None) -> NDArray[np.float64] | float:
        """Read variable ``name``, or a selection of it, as float64 (a float if no dimension is left).

        ``selection`` follows ``ZarrArray.read``. The whole variable is decoded first.

        Raises FileNotFoundError for an unknown name.
        """
        ...

def open_netcdf3(path: str | PathLike[str]) -> NetCdf3File:
    """Open and read the NetCDF-3 file at ``path``.

    Raises OSError if the file cannot be read and TptPlanetaryError if it is
    not a NetCDF-3 file.
    """
    ...

def open_zarr(path: str | PathLike[str]) -> ZarrArray:
    """Open the Zarr array in the local directory ``path``.

    Raises FileNotFoundError if no array metadata is present.
    """
    ...


class SpkKernel:
    """A DE440-style SPK kernel held in memory (see ``open_spk``).

    Times are TDB as seconds past J2000. Directions are ICRF.
    """

    def state(
        self, target: int, center: int, et: float
    ) -> tuple[tuple[float, float, float], tuple[float, float, float]]:
        """Position (km) and velocity (km/s) of ``target`` relative to ``center``."""
        ...
    def astrometric(self, target: int, observer: int, et: float) -> dict[str, float]:
        """Astrometric direction: ``ra_deg``, ``dec_deg``, ``distance_km``, ``light_time_s``.

        Light-time corrected, without stellar aberration (as JPL Horizons' astrometric RA and Dec).
        """
        ...
    def orientation(
        self, body_frame: int, et: float
    ) -> tuple[tuple[float, float, float], tuple[float, float, float], tuple[float, float, float]]:
        """Rotation from J2000 axes to the body-fixed frame ``body_frame`` (3000 for ITRF93),
        from a binary PCK, as a row-major 3 x 3 tuple (SPICE's pxform)."""
        ...
    def apparent(self, target: int, observer: int, et: float) -> dict[str, float]:
        """Apparent direction: ``ra_deg``, ``dec_deg``, ``light_time_s``.

        The astrometric direction with the Sun's light deflection and the annual aberration applied.
        """
        ...

def open_spk(path: str | PathLike[str]) -> SpkKernel:
    """Open and read the SPK kernel at ``path``.

    Raises OSError if the file cannot be read and TptPlanetaryError if it is not an SPK file.
    Kernels are not bundled; obtain one from JPL (for example de440s.bsp).
    """
    ...

def et_from_tdb(jd_hi: float, jd_lo: float) -> float:
    """Seconds past J2000 TDB for a TDB Julian date given in two parts."""
    ...


class Hdf5File:
    """A read-only HDF5 file (default format, as h5py and netCDF-4 write it), held in memory.

    Groups, datasets and attributes are addressed by slash-separated paths such as ``/a/b``.
    Unsupported features (dense groups, version 4 layouts, variable-length strings) raise
    TptPlanetaryError.
    """

    def kind(self, path: str) -> Literal["group", "dataset", "other"]:
        """Kind of the object at ``path``."""
        ...
    def list(self, path: str) -> list[str]:
        """Names of the children of the group at ``path``, in file order."""
        ...
    def shape(self, path: str) -> tuple[int, ...]:
        """Shape of the dataset at ``path``."""
        ...
    def read(self, path: str) -> NDArray[np.float64] | float:
        """Read the dataset at ``path`` as float64 (a float if it is scalar)."""
        ...
    def attributes(self, path: str) -> dict[str, object]:
        """Attributes of the group or dataset at ``path``: numbers as float64, text as str."""
        ...

def open_hdf5(path: str | PathLike[str]) -> Hdf5File:
    """Open and read the HDF5 file at ``path``.

    Raises OSError if the file cannot be read and TptPlanetaryError if it is not an
    HDF5 file or uses an unsupported feature.
    """
    ...


class Satellite:
    """An SGP4/SDP4 satellite from a two-line element set (TEME frame, km and km/s).

    Orbits with a period of 225 minutes or more use the SDP4 deep-space terms.
    """

    @staticmethod
    def from_tle(line1: str, line2: str) -> Satellite:
        """Initialise from the two TLE lines (checksums are checked)."""
        ...
    @property
    def epoch(self) -> tuple[float, float]:
        """Epoch as a two-part Julian date ``(day, fraction)``."""
        ...
    def propagate(
        self, minutes: float
    ) -> tuple[tuple[float, float, float], tuple[float, float, float]]:
        """Position (km) and velocity (km/s) at ``minutes`` after the epoch."""
        ...
