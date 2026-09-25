#![deny(unsafe_code)]

use crate::logic::STEERING_RANGE;

const CM_PER_INCH: f64 = 2.54;

pub fn counts_for_full_lock(sens: f64, dpi_scale: f64, saturation: f64) -> Option<f64> {
    let gain = sens * dpi_scale;
    (gain > 0.0 && gain.is_finite()).then(|| STEERING_RANGE * saturation / gain)
}

pub fn cm_for_full_lock(sens: f64, dpi_scale: f64, saturation: f64, dpi: i32) -> Option<f64> {
    if dpi <= 0 {
        return None;
    }
    counts_for_full_lock(sens, dpi_scale, saturation).map(|c| c / f64::from(dpi) * CM_PER_INCH)
}

pub fn sens_for_cm(cm: f64, dpi_scale: f64, saturation: f64, dpi: i32) -> Option<f64> {
    if dpi <= 0 || cm.is_nan() || cm <= 0.0 || dpi_scale.is_nan() || dpi_scale <= 0.0 {
        return None;
    }
    let counts = cm / CM_PER_INCH * f64::from(dpi);
    Some(STEERING_RANGE * saturation / (counts * dpi_scale))
}

pub fn dpi_from_measurement(counts: i64, distance_cm: f64) -> Option<i32> {
    if distance_cm.is_nan() || distance_cm <= 0.0 {
        return None;
    }
    let dpi = (counts.unsigned_abs() as f64 / (distance_cm / CM_PER_INCH)).round();
    (dpi >= 1.0 && dpi <= f64::from(i32::MAX)).then_some(dpi as i32)
}
