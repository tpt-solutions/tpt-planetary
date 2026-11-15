"""Tests for JulianDate, UTC/TAI/TT conversion, the leap-second table and angular separation."""

import numpy as np
import pytest

import tpt_planetary as tp


def test_j2000_round_trip_calendar():
    jd = tp.JulianDate.from_calendar(2000, 1, 1, 43_200.0)
    assert jd.to_float() == 2_451_545.0
    assert jd.to_calendar() == (2000, 1, 1, 43_200.0)


def test_two_part_normalisation():
    jd = tp.JulianDate(2_451_545.0, 1.25)
    assert jd.day == 2_451_546.0
    assert jd.fraction == pytest.approx(0.25, abs=1e-15)


def test_fraction_keeps_precision_that_a_single_float_loses():
    a = tp.JulianDate(2_451_545.0, 0.0)
    b = a.add_seconds(1e-6)
    # One microsecond is 1.16e-11 days; a single float64 near 2.4e6 resolves
    # only about 4.7e-10 days, so the two-part form is what keeps this exact.
    assert b.seconds_since(a) == pytest.approx(1e-6, abs=1e-9)


def test_calendar_julian_and_gregorian_differ_at_reform():
    # The Gregorian calendar here is proleptic, so 1582-10-05 exists in both.
    # Julian 1582-10-05 is Gregorian 1582-10-15, ten days later.
    jul = tp.JulianDate.from_calendar(1582, 10, 5, 0.0, calendar="julian")
    greg = tp.JulianDate.from_calendar(1582, 10, 5, 0.0, calendar="gregorian")
    assert jul.to_calendar(calendar="julian")[:3] == (1582, 10, 5)
    assert jul.days_since(greg) == pytest.approx(10.0, abs=1e-9)
    assert greg.to_calendar(calendar="gregorian")[:3] == (1582, 10, 5)


def test_mjd_and_differences():
    a = tp.JulianDate.from_calendar(2000, 1, 1, 0.0)
    b = tp.JulianDate.from_calendar(2000, 1, 2, 0.0)
    assert b.days_since(a) == pytest.approx(1.0, abs=1e-12)
    assert a.seconds_since(b) == pytest.approx(-86_400.0, abs=1e-6)
    assert a.to_mjd()[0] + a.to_mjd()[1] == pytest.approx(51_544.0, abs=1e-9)


def test_utc_to_tai_offset_2017_is_37_seconds():
    utc = tp.JulianDate.from_calendar(2017, 6, 1, 0.0)
    tai = tp.utc_to_tai(utc)
    assert tai.seconds_since(utc) == pytest.approx(37.0, abs=1e-6)


def test_tai_to_utc_inverts_utc_to_tai():
    utc = tp.JulianDate.from_calendar(2010, 3, 15, 12 * 3600.0)
    back = tp.tai_to_utc(tp.utc_to_tai(utc))
    assert abs(back.seconds_since(utc)) < 1e-6


def test_tai_tt_fixed_offset():
    tai = tp.JulianDate.from_calendar(2000, 1, 1, 0.0)
    tt = tp.tai_to_tt(tai)
    assert tt.seconds_since(tai) == pytest.approx(32.184, abs=1e-6)
    assert tp.tt_to_tai(tt).seconds_since(tai) == pytest.approx(0.0, abs=1e-6)


def test_utc_to_tt_at_j2000_is_64_184_seconds():
    # TAI-UTC was 32 s in 2000, plus the fixed 32.184 s TT-TAI offset.
    utc = tp.JulianDate.from_calendar(2000, 1, 1, 43_200.0)
    tt = tp.utc_to_tt(utc)
    assert tt.seconds_since(utc) == pytest.approx(64.184, abs=1e-6)
    assert tp.tt_to_utc(tt).seconds_since(utc) == pytest.approx(0.0, abs=1e-6)


def test_utc_before_1972_is_value_error():
    with pytest.raises(ValueError):
        tp.utc_to_tai(tp.JulianDate.from_calendar(1970, 1, 1, 0.0))


def test_utc_after_table_expiry_raises_library_error():
    jd = tp.JulianDate.from_calendar(2030, 1, 1, 0.0)
    assert tp.leap_second_table_expired(jd) is True
    with pytest.raises(tp.TptPlanetaryError):
        tp.utc_to_tai(jd)
    assert issubclass(tp.TptPlanetaryError, Exception)


def test_leap_second_table_info():
    info = tp.leap_second_table_info()
    assert info["version"] == 1
    assert info["valid_until"] == (2027, 6, 28)
    assert info["entries"] == 28
    assert info["last_tai_minus_utc"] == 37
    assert "IERS" in info["source"]
    assert tp.leap_second_table_expired(tp.JulianDate.from_calendar(2020, 1, 1, 0.0)) is False


@pytest.mark.parametrize(
    "args",
    [
        (2001, 2, 29, 0.0),  # not a leap year
        (2001, 13, 1, 0.0),  # month out of range
        (2001, 1, 0, 0.0),  # day out of range
        (2001, 1, 1, 86_400.0),  # seconds must be < 86400
        (2001, 1, 1, float("nan")),
        (2**40, 1, 1, 0.0),  # year beyond i32
    ],
)
def test_invalid_calendar_input_is_value_error(args):
    with pytest.raises(ValueError):
        tp.JulianDate.from_calendar(*args)


def test_unknown_calendar_name_is_value_error():
    with pytest.raises(ValueError):
        tp.JulianDate.from_calendar(2000, 1, 1, 0.0, calendar="mars")


def test_non_finite_julian_date_is_value_error():
    with pytest.raises(ValueError):
        tp.JulianDate(float("inf"))
    with pytest.raises(ValueError):
        tp.JulianDate(2_451_545.0, float("nan"))


@pytest.mark.parametrize("value", [1e300, -1e300, 1e15, 9.2e18, -9.2e18])
def test_extreme_julian_dates_are_value_errors(value):
    # The core crate panics on these (i64 overflow); the binding must reject them.
    with pytest.raises(ValueError):
        tp.JulianDate(value)


def test_extreme_offsets_do_not_panic():
    jd = tp.JulianDate.from_calendar(2000, 1, 1, 0.0)
    for bad in (1e300, -1e300, 1e13):
        with pytest.raises(ValueError):
            jd.add_days(bad)
        with pytest.raises(ValueError):
            jd.add_seconds(bad * 86_400.0)


def test_extreme_calendar_years_do_not_panic():
    for year in (2**31 - 1, -(2**31)):
        try:
            jd = tp.JulianDate.from_calendar(year, 6, 15, 0.0)
            tp.utc_to_tt(jd)
        except (ValueError, tp.TptPlanetaryError):
            pass


def test_angular_separation_known_values():
    lon1 = np.array([0.0, 10.0, 0.0, 0.0])
    lat1 = np.array([0.0, 20.0, 0.0, 30.0])
    lon2 = np.array([90.0, 10.0, 0.0, 180.0])
    lat2 = np.array([0.0, 20.0, 90.0, -30.0])
    sep = tp.angular_separation(lon1, lat1, lon2, lat2)
    assert sep.dtype == np.float64
    np.testing.assert_allclose(sep, [90.0, 0.0, 90.0, 180.0], atol=1e-9)


def test_angular_separation_matches_numpy_haversine():
    rng = np.random.default_rng(1)
    n = 1000
    lon1, lon2 = rng.uniform(0, 360, n), rng.uniform(0, 360, n)
    lat1, lat2 = rng.uniform(-90, 90, n), rng.uniform(-90, 90, n)
    d = np.deg2rad
    hav = np.sin((d(lat2) - d(lat1)) / 2) ** 2 + np.cos(d(lat1)) * np.cos(d(lat2)) * np.sin(
        (d(lon2) - d(lon1)) / 2
    ) ** 2
    expected = np.rad2deg(2 * np.arcsin(np.sqrt(np.clip(hav, 0, 1))))
    got = tp.angular_separation(lon1, lat1, lon2, lat2)
    np.testing.assert_allclose(got, expected, rtol=0, atol=1e-9)


def test_angular_separation_rejects_mismatched_lengths():
    a = np.zeros(3)
    with pytest.raises(ValueError):
        tp.angular_separation(a, a, np.zeros(2), a)


def test_angular_separation_rejects_wrong_dtype():
    a = np.zeros(3, dtype=np.float32)
    with pytest.raises(TypeError):
        tp.angular_separation(a, a, a, a)
