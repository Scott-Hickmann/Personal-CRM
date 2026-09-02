use rusqlite::{Connection, params};
use serde::Serialize;

use crate::error::Result;

const REGULARIZATION: f64 = 2.0;
const MIN_RATINGS: usize = 5;

#[derive(Clone, Debug, Serialize)]
pub struct Calibration {
    pub rating_count: usize,
    pub intercept: f64,
    pub scale: f64,
}

pub struct CalibrationPoint {
    pub base_score: f64,
    pub rating: i64,
}

impl Default for Calibration {
    fn default() -> Self {
        Self {
            rating_count: 0,
            intercept: 0.0,
            scale: 1.0,
        }
    }
}

impl Calibration {
    pub fn fit(points: &[CalibrationPoint]) -> Self {
        if points.len() < MIN_RATINGS {
            return Self {
                rating_count: points.len(),
                ..Self::default()
            };
        }
        let mut a00 = REGULARIZATION * 0.1;
        let mut a01 = 0.0;
        let mut a11 = REGULARIZATION;
        let mut b0 = 0.0;
        let mut b1 = REGULARIZATION;
        for point in points {
            let x = (point.base_score / 100.0).clamp(0.0, 1.0);
            let y = target_score(point.rating) / 100.0;
            a00 += 1.0;
            a01 += x;
            a11 += x * x;
            b0 += y;
            b1 += x * y;
        }
        let determinant = a00 * a11 - a01 * a01;
        if determinant.abs() < f64::EPSILON {
            return Self::default();
        }
        Self {
            rating_count: points.len(),
            intercept: ((b0 * a11 - b1 * a01) / determinant).clamp(-0.5, 0.5),
            scale: ((a00 * b1 - a01 * b0) / determinant).clamp(0.25, 2.0),
        }
    }

    pub fn apply(&self, base_score: f64) -> f64 {
        ((self.intercept + self.scale * base_score / 100.0) * 100.0).clamp(0.0, 100.0)
    }
}

pub fn target_score(rating: i64) -> f64 {
    match rating {
        1 => 5.0,
        2 => 15.0,
        3 => 30.0,
        4 => 50.0,
        5 => 65.0,
        6 => 75.0,
        7 => 90.0,
        _ => 50.0,
    }
}

pub fn set_rating(connection: &Connection, person_id: &str, rating: u8) -> Result<()> {
    connection.execute(
        "INSERT INTO affinity_ratings(person_id, rating) VALUES (?1, ?2)
         ON CONFLICT(person_id) DO UPDATE SET rating=excluded.rating,
         updated_at=CURRENT_TIMESTAMP",
        params![person_id, rating],
    )?;
    Ok(())
}

pub fn clear_rating(connection: &Connection, person_id: &str) -> Result<()> {
    connection.execute(
        "DELETE FROM affinity_ratings WHERE person_id=?1",
        [person_id],
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_ratings_preserves_the_base_score() {
        let calibration = Calibration::fit(&[]);
        assert_eq!(calibration.apply(63.0), 63.0);
    }

    #[test]
    fn ratings_shift_scores_monotonically() {
        let calibration = Calibration::fit(&[
            CalibrationPoint {
                base_score: 20.0,
                rating: 2,
            },
            CalibrationPoint {
                base_score: 70.0,
                rating: 7,
            },
            CalibrationPoint {
                base_score: 30.0,
                rating: 3,
            },
            CalibrationPoint {
                base_score: 50.0,
                rating: 5,
            },
            CalibrationPoint {
                base_score: 60.0,
                rating: 6,
            },
        ]);

        assert_eq!(calibration.rating_count, 5);
        assert!(calibration.apply(70.0) > calibration.apply(20.0));
    }

    #[test]
    fn sparse_ratings_anchor_only_the_rated_people() {
        let calibration = Calibration::fit(&[CalibrationPoint {
            base_score: 20.0,
            rating: 7,
        }]);

        assert_eq!(calibration.rating_count, 1);
        assert_eq!(calibration.apply(40.0), 40.0);
    }

    #[test]
    fn rating_targets_match_the_named_tiers() {
        assert_eq!(target_score(1), 5.0);
        assert_eq!(target_score(4), 50.0);
        assert_eq!(target_score(7), 90.0);
    }
}
