//! Deep-space (SDP4) terms for the SGP4 propagator, for orbits with a period
//! of 225 minutes or more.
//!
//! The routines follow the structure and constants of Vallado, Crawford,
//! Hujsak and Kelso (2006, AIAA 2006-6753), as in the reference `sgp4` Python
//! package (MIT licence, Brandon Rhodes). That package evaluates the resonance
//! integrator without keeping its state between calls, so each propagation is
//! independent of earlier ones; [`super::Sgp4::propagate`] keeps that property.
//!
//! Resonance is detected from the mean motion: the one-day (`irez = 1`) and
//! half-day (`irez = 2`) cases are integrated; other deep-space orbits get the
//! secular and lunar-solar periodic terms only.

use core::f64::consts::{PI, TAU};

const ZNS: f64 = 1.19459e-5;
const ZES: f64 = 0.01675;
const ZNL: f64 = 1.5835218e-4;
const ZEL: f64 = 0.05490;
// Full published digits of the reference constant (rad/min per minute of Earth rotation).
#[allow(clippy::excessive_precision)]
const RPTIM: f64 = 4.375_269_088_011_299_66e-3;
const DEG2RAD: f64 = PI / 180.0;

/// Deep-space quantities fixed at initialisation.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) struct DeepState {
    // Lunar-solar periodic coefficients (dscom), used by `dpper`.
    e3: f64,
    ee2: f64,
    se2: f64,
    se3: f64,
    sgh2: f64,
    sgh3: f64,
    sgh4: f64,
    sh2: f64,
    sh3: f64,
    si2: f64,
    si3: f64,
    sl2: f64,
    sl3: f64,
    sl4: f64,
    xgh2: f64,
    xgh3: f64,
    xgh4: f64,
    xh2: f64,
    xh3: f64,
    xi2: f64,
    xi3: f64,
    xl2: f64,
    xl3: f64,
    xl4: f64,
    zmol: f64,
    zmos: f64,
    // Resonance coefficients (dsinit), used by `dspace`.
    irez: u8,
    d2201: f64,
    d2211: f64,
    d3210: f64,
    d3222: f64,
    d4410: f64,
    d4422: f64,
    d5220: f64,
    d5232: f64,
    d5421: f64,
    d5433: f64,
    dedt: f64,
    didt: f64,
    dmdt: f64,
    dnodt: f64,
    domdt: f64,
    del1: f64,
    del2: f64,
    del3: f64,
    xfact: f64,
    xlamo: f64,
    /// Sidereal time at epoch, radians (SGP4's `gstime`).
    gsto: f64,
}

/// SGP4's Greenwich sidereal time (radians) at a UT1 Julian date. This is the
/// Vallado formula used by the reference code, not the IAU 2006 GMST.
pub(super) fn gstime(jdut1: f64) -> f64 {
    let tut1 = (jdut1 - 2_451_545.0) / 36_525.0;
    let temp = -6.2e-6 * tut1 * tut1 * tut1
        + 0.093_104 * tut1 * tut1
        + (876_600.0 * 3600.0 + 8_640_184.812_866) * tut1
        + 67_310.548_41;
    let temp = (temp * DEG2RAD / 240.0).rem_euclid(TAU);
    if temp < 0.0 { temp + TAU } else { temp }
}

/// Terms of one `dscom` pass: the solar (first) or lunar (second) contribution
/// to the short-period coefficients.
#[derive(Debug, Clone, Copy, Default)]
struct Pass {
    s1: f64,
    s2: f64,
    s3: f64,
    s4: f64,
    s5: f64,
    s6: f64,
    s7: f64,
    z1: f64,
    z2: f64,
    z3: f64,
    z11: f64,
    z12: f64,
    z13: f64,
    z21: f64,
    z22: f64,
    z23: f64,
    z31: f64,
    z32: f64,
    z33: f64,
}

/// Orientation of the Sun or Moon in the orbit frame, for one `dscom` pass.
#[derive(Debug, Clone, Copy)]
struct Orientation {
    zcosg: f64,
    zsing: f64,
    zcosi: f64,
    zsini: f64,
    zcosh: f64,
    zsinh: f64,
    cc: f64,
}

/// One pass of `dscom`'s coefficient loop.
#[allow(clippy::too_many_arguments)]
fn dscom_pass(
    o: Orientation,
    sinim: f64,
    cosim: f64,
    sinomm: f64,
    cosomm: f64,
    em: f64,
    emsq: f64,
    betasq: f64,
    rtemsq: f64,
    xnoi: f64,
) -> Pass {
    let a1 = o.zcosg * o.zcosh + o.zsing * o.zcosi * o.zsinh;
    let a3 = -o.zsing * o.zcosh + o.zcosg * o.zcosi * o.zsinh;
    let a7 = -o.zcosg * o.zsinh + o.zsing * o.zcosi * o.zcosh;
    let a8 = o.zsing * o.zsini;
    let a9 = o.zsing * o.zsinh + o.zcosg * o.zcosi * o.zcosh;
    let a10 = o.zcosg * o.zsini;
    let a2 = cosim * a7 + sinim * a8;
    let a4 = cosim * a9 + sinim * a10;
    let a5 = -sinim * a7 + cosim * a8;
    let a6 = -sinim * a9 + cosim * a10;

    let x1 = a1 * cosomm + a2 * sinomm;
    let x2 = a3 * cosomm + a4 * sinomm;
    let x3 = -a1 * sinomm + a2 * cosomm;
    let x4 = -a3 * sinomm + a4 * cosomm;
    let x5 = a5 * sinomm;
    let x6 = a6 * sinomm;
    let x7 = a5 * cosomm;
    let x8 = a6 * cosomm;

    let z31 = 12.0 * x1 * x1 - 3.0 * x3 * x3;
    let z32 = 24.0 * x1 * x2 - 6.0 * x3 * x4;
    let z33 = 12.0 * x2 * x2 - 3.0 * x4 * x4;
    let mut z1 = 3.0 * (a1 * a1 + a2 * a2) + z31 * emsq;
    let mut z2 = 6.0 * (a1 * a3 + a2 * a4) + z32 * emsq;
    let mut z3 = 3.0 * (a3 * a3 + a4 * a4) + z33 * emsq;
    let z11 = -6.0 * a1 * a5 + emsq * (-24.0 * x1 * x7 - 6.0 * x3 * x5);
    let z12 = -6.0 * (a1 * a6 + a3 * a5)
        + emsq * (-24.0 * (x2 * x7 + x1 * x8) - 6.0 * (x3 * x6 + x4 * x5));
    let z13 = -6.0 * a3 * a6 + emsq * (-24.0 * x2 * x8 - 6.0 * x4 * x6);
    let z21 = 6.0 * a2 * a5 + emsq * (24.0 * x1 * x5 - 6.0 * x3 * x7);
    let z22 =
        6.0 * (a4 * a5 + a2 * a6) + emsq * (24.0 * (x2 * x5 + x1 * x6) - 6.0 * (x4 * x7 + x3 * x8));
    let z23 = 6.0 * a4 * a6 + emsq * (24.0 * x2 * x6 - 6.0 * x4 * x8);
    z1 = z1 + z1 + betasq * z31;
    z2 = z2 + z2 + betasq * z32;
    z3 = z3 + z3 + betasq * z33;

    let s3 = o.cc * xnoi;
    let s2 = -0.5 * s3 / rtemsq;
    let s4 = s3 * rtemsq;
    let s1 = -15.0 * em * s4;
    let s5 = x1 * x3 + x2 * x4;
    let s6 = x2 * x3 + x1 * x4;
    let s7 = x2 * x4 - x1 * x3;

    Pass {
        s1,
        s2,
        s3,
        s4,
        s5,
        s6,
        s7,
        z1,
        z2,
        z3,
        z11,
        z12,
        z13,
        z21,
        z22,
        z23,
        z31,
        z32,
        z33,
    }
}

/// Coefficients shared by `dpper` and `dsinit` (`dscom`). The first pass (solar)
/// gives the `ss` and `sz` values; the second (lunar) gives `s` and `z`.
#[derive(Debug, Clone, Copy)]
struct Common {
    sinim: f64,
    cosim: f64,
    emsq: f64,
    p1: Pass,
    p2: Pass,
    // Lunar-solar periodic coefficients (see `DeepState`).
    se2: f64,
    se3: f64,
    si2: f64,
    si3: f64,
    sl2: f64,
    sl3: f64,
    sl4: f64,
    sgh2: f64,
    sgh3: f64,
    sgh4: f64,
    sh2: f64,
    sh3: f64,
    xi2: f64,
    xi3: f64,
    xl2: f64,
    xl3: f64,
    xl4: f64,
    xgh2: f64,
    xgh3: f64,
    xgh4: f64,
    xh2: f64,
    xh3: f64,
    e3: f64,
    ee2: f64,
    zmol: f64,
    zmos: f64,
}

/// Deep-space common items (`dscom`) for an epoch, with `epoch` in days since
/// 1949 December 31 0h, the eccentricity `ep`, the argument of perigee
/// `argpp`, the inclination `inclp`, the node `nodep` and the mean motion `np`
/// in radians per minute. The reference uses `tc = 0` at initialisation.
fn dscom(epoch: f64, ep: f64, argpp: f64, inclp: f64, nodep: f64, np: f64) -> Common {
    let zes = 0.01675;
    let zel = 0.05490;
    let c1ss = 2.986_479_7e-6;
    let c1l = 4.796_806_5e-7;
    let zsinis = 0.397_854_16;
    let zcosis = 0.917_448_67;
    let zcosgs = 0.194_590_5;
    let zsings = -0.980_884_58;

    let nm = np;
    let em = ep;
    let snodm = nodep.sin();
    let cnodm = nodep.cos();
    let sinomm = argpp.sin();
    let cosomm = argpp.cos();
    let sinim = inclp.sin();
    let cosim = inclp.cos();
    let emsq = em * em;
    let betasq = 1.0 - emsq;
    let rtemsq = betasq.sqrt();

    let day = epoch + 18_261.5;
    let xnodce = (4.523_602_0 - 9.242_202_9e-4 * day).rem_euclid(TAU);
    let stem = xnodce.sin();
    let ctem = xnodce.cos();
    let zcosil = 0.913_751_64 - 0.035_680_96 * ctem;
    let zsinil = (1.0 - zcosil * zcosil).sqrt();
    let zsinhl = 0.089_683_511 * stem / zsinil;
    let zcoshl = (1.0 - zsinhl * zsinhl).sqrt();
    let gam = 5.835_151_4 + 0.001_944_368_0 * day;
    let mut zx = 0.397_854_16 * stem / zsinil;
    let zy = zcoshl * ctem + 0.917_448_67 * zsinhl * stem;
    zx = zx.atan2(zy);
    zx = gam + zx - xnodce;
    let zcosgl = zx.cos();
    let zsingl = zx.sin();

    let xnoi = 1.0 / nm;
    let first = Orientation {
        zcosg: zcosgs,
        zsing: zsings,
        zcosi: zcosis,
        zsini: zsinis,
        zcosh: cnodm,
        zsinh: snodm,
        cc: c1ss,
    };
    let p1 = dscom_pass(
        first, sinim, cosim, sinomm, cosomm, em, emsq, betasq, rtemsq, xnoi,
    );
    let second = Orientation {
        zcosg: zcosgl,
        zsing: zsingl,
        zcosi: zcosil,
        zsini: zsinil,
        zcosh: zcoshl * cnodm + zsinhl * snodm,
        zsinh: snodm * zcoshl - cnodm * zsinhl,
        cc: c1l,
    };
    let p2 = dscom_pass(
        second, sinim, cosim, sinomm, cosomm, em, emsq, betasq, rtemsq, xnoi,
    );

    let zmol = (4.719_967_2 + 0.229_971_50 * day - gam).rem_euclid(TAU);
    let zmos = (6.256_583_7 + 0.017_201_977 * day).rem_euclid(TAU);

    let (ss1, ss2, ss3, ss4, ss6, ss7) = (p1.s1, p1.s2, p1.s3, p1.s4, p1.s6, p1.s7);
    let (sz1, sz2, sz3) = (p1.z1, p1.z2, p1.z3);
    let (sz11, sz12, sz13) = (p1.z11, p1.z12, p1.z13);
    let (sz21, sz22, sz23) = (p1.z21, p1.z22, p1.z23);
    let (sz31, sz32, sz33) = (p1.z31, p1.z32, p1.z33);
    let Pass {
        s1,
        s2,
        s3,
        s4,
        s6,
        s7,
        z1,
        z2,
        z3,
        z11,
        z12,
        z13,
        z21,
        z22,
        z23,
        z31,
        z32,
        z33,
        ..
    } = p2;

    let se2 = 2.0 * ss1 * ss6;
    let se3 = 2.0 * ss1 * ss7;
    let si2 = 2.0 * ss2 * sz12;
    let si3 = 2.0 * ss2 * (sz13 - sz11);
    let sl2 = -2.0 * ss3 * sz2;
    let sl3 = -2.0 * ss3 * (sz3 - sz1);
    let sl4 = -2.0 * ss3 * (-21.0 - 9.0 * emsq) * zes;
    let sgh2 = 2.0 * ss4 * sz32;
    let sgh3 = 2.0 * ss4 * (sz33 - sz31);
    let sgh4 = -18.0 * ss4 * zes;
    let sh2 = -2.0 * ss2 * sz22;
    let sh3 = -2.0 * ss2 * (sz23 - sz21);
    let ee2 = 2.0 * s1 * s6;
    let e3 = 2.0 * s1 * s7;
    let xi2 = 2.0 * s2 * z12;
    let xi3 = 2.0 * s2 * (z13 - z11);
    let xl2 = -2.0 * s3 * z2;
    let xl3 = -2.0 * s3 * (z3 - z1);
    let xl4 = -2.0 * s3 * (-21.0 - 9.0 * emsq) * zel;
    let xgh2 = 2.0 * s4 * z32;
    let xgh3 = 2.0 * s4 * (z33 - z31);
    let xgh4 = -18.0 * s4 * zel;
    let xh2 = -2.0 * s2 * z22;
    let xh3 = -2.0 * s2 * (z23 - z21);

    Common {
        sinim,
        cosim,
        emsq,
        p1,
        p2,
        se2,
        se3,
        si2,
        si3,
        sl2,
        sl3,
        sl4,
        sgh2,
        sgh3,
        sgh4,
        sh2,
        sh3,
        xi2,
        xi3,
        xl2,
        xl3,
        xl4,
        xgh2,
        xgh3,
        xgh4,
        xh2,
        xh3,
        e3,
        ee2,
        zmol,
        zmos,
    }
}

/// Deep-space initialisation (`sgp4init` for the deep-space case).
///
/// `epoch` is days since 1949 December 31 0h. `no` is the un-Kozai mean
/// motion (radians per minute); `gsto` is the sidereal time at epoch.
#[allow(clippy::too_many_arguments)]
pub(super) fn initialise(
    xke: f64,
    epoch: f64,
    ecco: f64,
    eccsq: f64,
    argpo: f64,
    inclo: f64,
    mo: f64,
    mdot: f64,
    no: f64,
    nodeo: f64,
    nodedot: f64,
    argpdot: f64,
    gsto: f64,
) -> DeepState {
    let xpidot = argpdot + nodedot;
    let c = dscom(epoch, ecco, argpo, inclo, nodeo, no);
    let ds = dsinit(
        xke, &c, argpo, mo, mdot, no, nodeo, nodedot, xpidot, ecco, eccsq, gsto, inclo,
    );
    DeepState {
        e3: c.e3,
        ee2: c.ee2,
        se2: c.se2,
        se3: c.se3,
        sgh2: c.sgh2,
        sgh3: c.sgh3,
        sgh4: c.sgh4,
        sh2: c.sh2,
        sh3: c.sh3,
        si2: c.si2,
        si3: c.si3,
        sl2: c.sl2,
        sl3: c.sl3,
        sl4: c.sl4,
        xgh2: c.xgh2,
        xgh3: c.xgh3,
        xgh4: c.xgh4,
        xh2: c.xh2,
        xh3: c.xh3,
        xi2: c.xi2,
        xi3: c.xi3,
        xl2: c.xl2,
        xl3: c.xl3,
        xl4: c.xl4,
        zmol: c.zmol,
        zmos: c.zmos,
        irez: ds.irez,
        d2201: ds.d2201,
        d2211: ds.d2211,
        d3210: ds.d3210,
        d3222: ds.d3222,
        d4410: ds.d4410,
        d4422: ds.d4422,
        d5220: ds.d5220,
        d5232: ds.d5232,
        d5421: ds.d5421,
        d5433: ds.d5433,
        dedt: ds.dedt,
        didt: ds.didt,
        dmdt: ds.dmdt,
        dnodt: ds.dnodt,
        domdt: ds.domdt,
        del1: ds.del1,
        del2: ds.del2,
        del3: ds.del3,
        xfact: ds.xfact,
        xlamo: ds.xlamo,
        gsto,
    }
}

/// Resonance and secular rates and coefficients (`dsinit`) at initialisation,
/// where `t = 0` and `tc = 0`.
#[derive(Debug, Clone, Copy, Default)]
struct DsInit {
    irez: u8,
    d2201: f64,
    d2211: f64,
    d3210: f64,
    d3222: f64,
    d4410: f64,
    d4422: f64,
    d5220: f64,
    d5232: f64,
    d5421: f64,
    d5433: f64,
    dedt: f64,
    didt: f64,
    dmdt: f64,
    dnodt: f64,
    domdt: f64,
    del1: f64,
    del2: f64,
    del3: f64,
    xfact: f64,
    xlamo: f64,
}

#[allow(clippy::too_many_arguments)]
fn dsinit(
    xke: f64,
    c: &Common,
    argpo: f64,
    mo: f64,
    mdot: f64,
    no: f64,
    nodeo: f64,
    nodedot: f64,
    xpidot: f64,
    ecco: f64,
    eccsq: f64,
    gsto: f64,
    inclm: f64,
) -> DsInit {
    let q22 = 1.789_167_9e-6;
    let q31 = 2.146_074_8e-6;
    let q33 = 2.212_301_5e-7;
    let root22 = 1.789_167_9e-6;
    let root44 = 7.363_695_3e-9;
    let root54 = 2.176_580_3e-9;
    let root32 = 3.739_379_2e-7;
    let root52 = 1.142_863_9e-7;
    let x2o3 = 2.0 / 3.0;

    let nm = no;
    let (s1, s2, s3, s4, s5) = (c.p2.s1, c.p2.s2, c.p2.s3, c.p2.s4, c.p2.s5);
    let (ss1, ss2, ss3, ss4, ss5) = (c.p1.s1, c.p1.s2, c.p1.s3, c.p1.s4, c.p1.s5);
    let (sz1, sz3) = (c.p1.z1, c.p1.z3);
    let (sz11, sz13, sz21, sz23, sz31, sz33) =
        (c.p1.z11, c.p1.z13, c.p1.z21, c.p1.z23, c.p1.z31, c.p1.z33);
    let (z1, z3, z11, z13, z21, z23, z31, z33) = (
        c.p2.z1, c.p2.z3, c.p2.z11, c.p2.z13, c.p2.z21, c.p2.z23, c.p2.z31, c.p2.z33,
    );
    let sinim = c.sinim;
    let cosim = c.cosim;
    let emsq = c.emsq;

    let mut out = DsInit::default();
    if 0.003_490_658_5 < nm && nm < 0.005_235_987_7 {
        out.irez = 1;
    }
    if (8.26e-3..=9.24e-3).contains(&nm) && ecco >= 0.5 {
        out.irez = 2;
    }

    // Solar and lunar secular rates.
    let ses = ss1 * ZNS * ss5;
    let sis = ss2 * ZNS * (sz11 + sz13);
    let sls = -ZNS * ss3 * (sz1 + sz3 - 14.0 - 6.0 * emsq);
    let sghs = ss4 * ZNS * (sz31 + sz33 - 6.0);
    let mut shs = -ZNS * ss2 * (sz21 + sz23);
    if !(0.052_359_877_7..=PI - 0.052_359_877_7).contains(&inclm) {
        shs = 0.0;
    }
    if sinim != 0.0 {
        shs /= sinim;
    }
    let sgs = sghs - cosim * shs;
    out.dedt = ses + s1 * ZNL * s5;
    out.didt = sis + s2 * ZNL * (z11 + z13);
    out.dmdt = sls - ZNL * s3 * (z1 + z3 - 14.0 - 6.0 * emsq);
    let sghl = s4 * ZNL * (z31 + z33 - 6.0);
    let mut shll = -ZNL * s2 * (z21 + z23);
    if !(0.052_359_877_7..=PI - 0.052_359_877_7).contains(&inclm) {
        shll = 0.0;
    }
    out.domdt = sgs + sghl;
    out.dnodt = shs;
    if sinim != 0.0 {
        out.domdt -= cosim / sinim * shll;
        out.dnodt += shll / sinim;
    }
    let theta = (gsto).rem_euclid(TAU);

    if out.irez != 0 {
        let aonv = (nm / xke).powf(x2o3);
        if out.irez == 2 {
            let cosisq = cosim * cosim;
            let em = ecco;
            let emsq_i = eccsq;
            let eoc = em * emsq_i;
            let g201 = -0.306 - (em - 0.64) * 0.440;
            let (g211, g310, g322, g410, g422, g520);
            if em <= 0.65 {
                g211 = 3.616 - 13.2470 * em + 16.2900 * emsq_i;
                g310 = -19.302 + 117.3900 * em - 228.4190 * emsq_i + 156.5910 * eoc;
                g322 = -18.9068 + 109.7927 * em - 214.6334 * emsq_i + 146.5816 * eoc;
                g410 = -41.122 + 242.6940 * em - 471.0940 * emsq_i + 313.9530 * eoc;
                g422 = -146.407 + 841.8800 * em - 1629.014 * emsq_i + 1083.4350 * eoc;
                g520 = -532.114 + 3017.977 * em - 5740.032 * emsq_i + 3708.2760 * eoc;
            } else {
                g211 = -72.099 + 331.819 * em - 508.738 * emsq_i + 266.724 * eoc;
                g310 = -346.844 + 1582.851 * em - 2415.925 * emsq_i + 1246.113 * eoc;
                g322 = -342.585 + 1554.908 * em - 2366.899 * emsq_i + 1215.972 * eoc;
                g410 = -1052.797 + 4758.686 * em - 7193.992 * emsq_i + 3651.957 * eoc;
                g422 = -3581.690 + 16178.110 * em - 24462.770 * emsq_i + 12422.520 * eoc;
                g520 = if em > 0.715 {
                    -5149.66 + 29936.92 * em - 54087.36 * emsq_i + 31324.56 * eoc
                } else {
                    1464.74 - 4664.75 * em + 3763.64 * emsq_i
                };
            }
            let (g533, g521, g532);
            if em < 0.7 {
                g533 = -919.227_70 + 4988.6100 * em - 9064.7700 * emsq_i + 5542.21 * eoc;
                g521 = -822.710_72 + 4568.6173 * em - 8491.4146 * emsq_i + 5337.524 * eoc;
                g532 = -853.666_00 + 4690.2500 * em - 8624.7700 * emsq_i + 5341.4 * eoc;
            } else {
                g533 = -37995.780 + 161616.52 * em - 229838.20 * emsq_i + 109377.94 * eoc;
                g521 = -51752.104 + 218913.95 * em - 309468.16 * emsq_i + 146349.42 * eoc;
                g532 = -40023.880 + 170470.89 * em - 242699.48 * emsq_i + 115605.82 * eoc;
            }
            let sini2 = sinim * sinim;
            let f220 = 0.75 * (1.0 + 2.0 * cosim + cosisq);
            let f221 = 1.5 * sini2;
            let f321 = 1.875 * sinim * (1.0 - 2.0 * cosim - 3.0 * cosisq);
            let f322 = -1.875 * sinim * (1.0 + 2.0 * cosim - 3.0 * cosisq);
            let f441 = 35.0 * sini2 * f220;
            let f442 = 39.375_0 * sini2 * sini2;
            let f522 = 9.843_75
                * sinim
                * (sini2 * (1.0 - 2.0 * cosim - 5.0 * cosisq)
                    + 0.333_333_33 * (-2.0 + 4.0 * cosim + 6.0 * cosisq));
            let f523 = sinim
                * (4.921_875_12 * sini2 * (-2.0 - 4.0 * cosim + 10.0 * cosisq)
                    + 6.562_500_12 * (1.0 + 2.0 * cosim - 3.0 * cosisq));
            let f542 = 29.531_25
                * sinim
                * (2.0 - 8.0 * cosim + cosisq * (-12.0 + 8.0 * cosim + 10.0 * cosisq));
            let f543 = 29.531_25
                * sinim
                * (-2.0 - 8.0 * cosim + cosisq * (12.0 + 8.0 * cosim - 10.0 * cosisq));
            let xno2 = nm * nm;
            let ainv2 = aonv * aonv;
            let mut temp1 = 3.0 * xno2 * ainv2;
            let mut temp = temp1 * root22;
            out.d2201 = temp * f220 * g201;
            out.d2211 = temp * f221 * g211;
            temp1 *= aonv;
            temp = temp1 * root32;
            out.d3210 = temp * f321 * g310;
            out.d3222 = temp * f322 * g322;
            temp1 *= aonv;
            temp = 2.0 * temp1 * root44;
            out.d4410 = temp * f441 * g410;
            out.d4422 = temp * f442 * g422;
            temp1 *= aonv;
            temp = temp1 * root52;
            out.d5220 = temp * f522 * g520;
            out.d5232 = temp * f523 * g532;
            temp = 2.0 * temp1 * root54;
            out.d5421 = temp * f542 * g521;
            out.d5433 = temp * f543 * g533;
            out.xlamo = (mo + nodeo + nodeo - theta - theta).rem_euclid(TAU);
            out.xfact = mdot + out.dmdt + 2.0 * (nodedot + out.dnodt - RPTIM) - no;
        }
        if out.irez == 1 {
            let emsq_i = emsq;
            let g200 = 1.0 + emsq_i * (-2.5 + 0.8125 * emsq_i);
            let g310 = 1.0 + 2.0 * emsq_i;
            let g300 = 1.0 + emsq_i * (-6.0 + 6.609_37 * emsq_i);
            let f220 = 0.75 * (1.0 + cosim) * (1.0 + cosim);
            let f311 = 0.937_5 * sinim * sinim * (1.0 + 3.0 * cosim) - 0.75 * (1.0 + cosim);
            let mut f330 = 1.0 + cosim;
            f330 = 1.875 * f330 * f330 * f330;
            out.del1 = 3.0 * nm * nm * aonv * aonv;
            out.del2 = 2.0 * out.del1 * f220 * g200 * q22;
            out.del3 = 3.0 * out.del1 * f330 * g300 * q33 * aonv;
            out.del1 = out.del1 * f311 * g310 * q31 * aonv;
            out.xlamo = (mo + nodeo + argpo - theta).rem_euclid(TAU);
            out.xfact = mdot + xpidot - RPTIM + out.dmdt + out.domdt + out.dnodt - no;
        }
    }
    out
}

/// Outputs of the resonance integrator `dspace` at one time.
#[derive(Debug, Clone, Copy)]
pub(super) struct Dspace {
    pub em: f64,
    pub argpm: f64,
    pub inclm: f64,
    pub mm: f64,
    pub nodem: f64,
    pub nm: f64,
}

/// Secular and resonance updates at `t` minutes after epoch (`dspace`). The
/// integration starts from the initial state at each call, as in the `sgp4`
/// package, so the result does not depend on earlier calls.
#[allow(clippy::too_many_arguments)]
pub(super) fn dspace(
    ds: &DeepState,
    argpo: f64,
    argpdot: f64,
    t: f64,
    no: f64,
    em0: f64,
    argpm0: f64,
    inclm0: f64,
    mm0: f64,
    nodem0: f64,
    nm0: f64,
) -> Dspace {
    let fasx2 = 0.131_309_08;
    let fasx4 = 2.884_319_8;
    let fasx6 = 0.374_480_87;
    let g22 = 5.768_639_6;
    let g32 = 0.952_408_98;
    let g44 = 1.801_499_8;
    let g52 = 1.050_833_0;
    let g54 = 4.410_889_8;
    let stepp = 720.0;
    let stepn = -720.0;
    let step2 = 259_200.0;

    let theta = (ds.gsto + t * RPTIM).rem_euclid(TAU);
    let em = em0 + ds.dedt * t;
    let inclm = inclm0 + ds.didt * t;
    let argpm = argpm0 + ds.domdt * t;
    let nodem = nodem0 + ds.dnodt * t;
    let mut mm = mm0 + ds.dmdt * t;
    let mut nm = nm0;

    if ds.irez != 0 {
        // The integrator starts at the epoch state (atime = 0).
        let mut atime = 0.0;
        let mut xli = ds.xlamo;
        let mut xni = no;
        let delt = if t > 0.0 { stepp } else { stepn };
        let ft;
        let (mut xndt, mut xldot, mut xnddt);
        loop {
            if ds.irez != 2 {
                xndt = ds.del1 * (xli - fasx2).sin()
                    + ds.del2 * (2.0 * (xli - fasx4)).sin()
                    + ds.del3 * (3.0 * (xli - fasx6)).sin();
                xldot = xni + ds.xfact;
                xnddt = ds.del1 * (xli - fasx2).cos()
                    + 2.0 * ds.del2 * (2.0 * (xli - fasx4)).cos()
                    + 3.0 * ds.del3 * (3.0 * (xli - fasx6)).cos();
                xnddt *= xldot;
            } else {
                let xomi = argpo + argpdot * atime;
                let x2omi = xomi + xomi;
                let x2li = xli + xli;
                xndt = ds.d2201 * (x2omi + xli - g22).sin()
                    + ds.d2211 * (xli - g22).sin()
                    + ds.d3210 * (xomi + xli - g32).sin()
                    + ds.d3222 * (-xomi + xli - g32).sin()
                    + ds.d4410 * (x2omi + x2li - g44).sin()
                    + ds.d4422 * (x2li - g44).sin()
                    + ds.d5220 * (xomi + xli - g52).sin()
                    + ds.d5232 * (-xomi + xli - g52).sin()
                    + ds.d5421 * (xomi + x2li - g54).sin()
                    + ds.d5433 * (-xomi + x2li - g54).sin();
                xldot = xni + ds.xfact;
                xnddt = ds.d2201 * (x2omi + xli - g22).cos()
                    + ds.d2211 * (xli - g22).cos()
                    + ds.d3210 * (xomi + xli - g32).cos()
                    + ds.d3222 * (-xomi + xli - g32).cos()
                    + ds.d5220 * (xomi + xli - g52).cos()
                    + ds.d5232 * (-xomi + xli - g52).cos()
                    + 2.0
                        * (ds.d4410 * (x2omi + x2li - g44).cos()
                            + ds.d4422 * (x2li - g44).cos()
                            + ds.d5421 * (xomi + x2li - g54).cos()
                            + ds.d5433 * (-xomi + x2li - g54).cos());
                xnddt *= xldot;
            }
            if (t - atime).abs() >= stepp {
                xli += xldot * delt + xndt * step2;
                xni += xndt * delt + xnddt * step2;
                atime += delt;
            } else {
                ft = t - atime;
                break;
            }
        }
        nm = xni + xndt * ft + xnddt * ft * ft * 0.5;
        let xl = xli + xldot * ft + xndt * ft * ft * 0.5;
        if ds.irez != 1 {
            mm = xl - 2.0 * nodem + 2.0 * theta;
        } else {
            mm = xl - nodem - argpm + theta;
        }
        let dndt = nm - no;
        nm = no + dndt;
    }
    Dspace {
        em,
        argpm,
        inclm,
        mm,
        nodem,
        nm,
    }
}

/// Lunar-solar periodic corrections (`dpper`) to the eccentricity, inclination,
/// node, argument of perigee and mean anomaly, at `t` minutes after epoch.
///
/// With `init` set (initialisation) the epoch values are returned unchanged,
/// as in the reference. The `opsmode 'i'` branch is used throughout.
#[allow(clippy::too_many_arguments)]
pub(super) fn dpper(
    ds: &DeepState,
    t: f64,
    init: bool,
    ep: f64,
    inclp: f64,
    nodep: f64,
    argpp: f64,
    mp: f64,
) -> (f64, f64, f64, f64, f64) {
    let zm0 = if init { ds.zmos } else { ds.zmos + ZNS * t };
    let zf = zm0 + 2.0 * ZES * zm0.sin();
    let sinzf = zf.sin();
    let f2 = 0.5 * sinzf * sinzf - 0.25;
    let f3 = -0.5 * sinzf * zf.cos();
    let ses = ds.se2 * f2 + ds.se3 * f3;
    let sis = ds.si2 * f2 + ds.si3 * f3;
    let sls = ds.sl2 * f2 + ds.sl3 * f3 + ds.sl4 * sinzf;
    let sghs = ds.sgh2 * f2 + ds.sgh3 * f3 + ds.sgh4 * sinzf;
    let shs = ds.sh2 * f2 + ds.sh3 * f3;

    let zm1 = if init { ds.zmol } else { ds.zmol + ZNL * t };
    let zf = zm1 + 2.0 * ZEL * zm1.sin();
    let sinzf = zf.sin();
    let f2 = 0.5 * sinzf * sinzf - 0.25;
    let f3 = -0.5 * sinzf * zf.cos();
    let sel = ds.ee2 * f2 + ds.e3 * f3;
    let sil = ds.xi2 * f2 + ds.xi3 * f3;
    let sll = ds.xl2 * f2 + ds.xl3 * f3 + ds.xl4 * sinzf;
    let sghl = ds.xgh2 * f2 + ds.xgh3 * f3 + ds.xgh4 * sinzf;
    let shll = ds.xh2 * f2 + ds.xh3 * f3;

    let pe = ses + sel;
    let pinc = sis + sil;
    let pl = sls + sll;
    let mut pgh = sghs + sghl;
    let mut ph = shs + shll;

    if init {
        return (ep, inclp, nodep, argpp, mp);
    }
    // The epoch values of the periodics are zero, so nothing is subtracted.
    let inclp = inclp + pinc;
    let ep = ep + pe;
    let sinip = inclp.sin();
    let cosip = inclp.cos();

    // Apply the periodics; for low inclinations use the Lyddane-type form.
    if inclp >= 0.2 {
        ph /= sinip;
        pgh -= cosip * ph;
        let argpp = argpp + pgh;
        let nodep = nodep + ph;
        let mp = mp + pl;
        (ep, inclp, nodep, argpp, mp)
    } else {
        let sinop = nodep.sin();
        let cosop = nodep.cos();
        let mut alfdp = sinip * sinop;
        let mut betdp = sinip * cosop;
        let dalf = ph * cosop + pinc * cosip * sinop;
        let dbet = -ph * sinop + pinc * cosip * cosop;
        alfdp += dalf;
        betdp += dbet;
        let nodep = if nodep >= 0.0 {
            nodep.rem_euclid(TAU)
        } else {
            -((-nodep).rem_euclid(TAU))
        };
        let xls = mp + argpp + pl + pgh + (cosip - pinc * sinip) * nodep;
        let xnoh = nodep;
        let mut nodep = alfdp.atan2(betdp);
        if (xnoh - nodep).abs() > PI {
            if nodep < xnoh {
                nodep += TAU;
            } else {
                nodep -= TAU;
            }
        }
        let mp = mp + pl;
        let argpp = xls - mp - cosip * nodep;
        (ep, inclp, nodep, argpp, mp)
    }
}
