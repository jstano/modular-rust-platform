use stano_route_macros::get;

#[get(path = "/health")]
struct NotAFn;

fn main() {}
