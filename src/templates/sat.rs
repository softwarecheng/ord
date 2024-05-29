use super::*;

#[derive(Boilerplate)]
pub(crate) struct SatHtml {
  pub(crate) sat: Sat,
  pub(crate) satpoint: Option<SatPoint>,
  pub(crate) blocktime: Blocktime,
  pub(crate) inscriptions: Vec<InscriptionId>,
}

impl PageContent for SatHtml {
  fn title(&self) -> String {
    format!("Sat {}", self.sat)
  }
}


