use stano_route_macros::get;

struct AppJson<T>(T);
struct Foo;
struct Bar;

#[get(path = "/x", responses((status = 200, body = Bar)))]
async fn handler() -> AppJson<Foo> {
    todo!()
}

fn main() {}
