use super::*;

#[derive(Boilerplate)]
pub(crate) struct ClockSvg {
  height: Height,
  hour: f64,
  minute: f64,
  second: f64,
}

impl ClockSvg {
  pub(crate) fn new(height: Height) -> Self {
    let min = height.min(Epoch::FIRST_POST_SUBSIDY.starting_height());

    Self {
      height,
      hour: f64::from(min.n() % Epoch::FIRST_POST_SUBSIDY.starting_height().n())
        / f64::from(Epoch::FIRST_POST_SUBSIDY.starting_height().n())
        * 360.0,
      minute: f64::from(min.n() % SUBSIDY_HALVING_INTERVAL) / f64::from(SUBSIDY_HALVING_INTERVAL)
        * 360.0,
      second: f64::from(height.period_offset()) / f64::from(DIFFCHANGE_INTERVAL) * 360.0,
    }
  }
}

