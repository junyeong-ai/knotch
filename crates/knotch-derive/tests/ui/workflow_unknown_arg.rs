use knotch_derive::workflow;

// `bogus` is not a recognized #[workflow] key — the macro must reject
// it with a clear error.
#[workflow(name = "x", phase = (), milestone = (), gate = (), bogus = 1)]
pub struct Flow;

fn main() {}
