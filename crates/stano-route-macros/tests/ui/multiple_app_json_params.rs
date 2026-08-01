use stano_route_macros::post;

struct AppJson<T>(T);
struct Foo;
struct Bar;
struct Baz;

#[post(path = "/x")]
async fn handler(a: AppJson<Foo>, b: AppJson<Bar>) -> AppJson<Baz> {
    todo!()
}

fn main() {}
