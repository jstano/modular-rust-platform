use stano_di_macros::component;

#[component]
trait Greeter {
    fn greet(&self) -> String;
}

fn main() {}
