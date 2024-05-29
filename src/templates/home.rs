use super::*;

#[derive(Boilerplate)]
pub(crate) struct HomeHtml {
  pub(crate) inscriptions: Vec<InscriptionId>,
}

impl PageContent for HomeHtml {
  fn title(&self) -> String {
    "Ordinals".to_string()
  }
}

