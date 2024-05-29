use super::*;

#[derive(Boilerplate)]
pub(crate) struct RangeHtml {
  pub(crate) start: Sat,
  pub(crate) end: Sat,
}

impl PageContent for RangeHtml {
  fn title(&self) -> String {
    format!("Sat Range {}–{}", self.start, self.end)
  }
}
