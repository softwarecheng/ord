use super::*;

#[derive(Boilerplate)]
pub(crate) struct CollectionsHtml {
  pub(crate) inscriptions: Vec<InscriptionId>,
  pub(crate) prev: Option<usize>,
  pub(crate) next: Option<usize>,
}

impl PageContent for CollectionsHtml {
  fn title(&self) -> String {
    "Collections".into()
  }
}

