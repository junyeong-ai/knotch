use knotch_derive::workflow;

// `name` is mandatory — omission must fail at compile time.
#[workflow(phase = (), milestone = (), gate = ())]
pub struct Flow;

fn main() {}
