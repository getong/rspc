#![allow(dead_code)]

use std::time::Instant;

use specta::Type;

use crate::ts::assert_ts;

struct Unsupported<T>(T);
struct Unsupported2;

#[test]
fn simple() {
    #[derive(Type)]
    struct Override {
        a: i32,
        #[specta(type = String)]
        x: Instant,
        #[specta(type = String)]
        y: Unsupported<Unsupported<Unsupported2>>,
        #[specta(type = Option<String>)]
        z: Option<Unsupported2>,
    }

    assert_ts!(
        Override,
        "{ a: number; x: string; y: string; z: string | null }"
    );
}

#[test]
fn newtype() {
    #[derive(Type)]
    struct New1(#[specta(type = String)] Unsupported2);
    #[derive(Type)]
    struct New2(#[specta(type = Option<String>)] Unsupported<Unsupported2>);

    assert_ts!(New1, r#"string"#);
    assert_ts!(New2, r#"string | null"#);
}
