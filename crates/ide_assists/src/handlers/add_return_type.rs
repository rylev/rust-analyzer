use crate::assist_context::{AssistContext, Assists};
use hir::{HirDisplay, TypeInfo};
use ide_db::assists::AssistId;
use syntax::{ast::Fn, AstNode};

// Assist: add_return_type
//
// Adds a return type annotation to function.
//
// ```
// fn foo() {
//     "Hello, world"
// }
// ```
// ->
// ```
// fn main() -> &'static str {
//     "Hello, world"
// }
// ```
pub(crate) fn add_return_type(acc: &mut Assists, ctx: &AssistContext) -> Option<()> {
    let fun = ctx.find_node_at_offset::<Fn>()?;
    let ret_type = fun.ret_type();
    let body = fun.body()?;
    let expr = body.tail_expr()?;
    let param_list = fun.param_list()?;
    let param_list = param_list.syntax();
    let module = ctx.sema.scope(body.syntax()).module()?;

    let inferred_type = ctx.sema.type_of_expr(&expr).map(TypeInfo::original)?;
    // TODO: compare ret_type and inferred_type and return None if their the same
    if ret_type.is_some() {
        return None;
    }
    let inferred_type = inferred_type.display_source_code(ctx.db(), module.into()).ok()?;
    acc.add(
        AssistId("add_return_type", crate::AssistKind::QuickFix),
        "Add a return type",
        param_list.text_range(),
        move |builder| {
            builder.insert(param_list.text_range().end(), format!(" -> {}", inferred_type))
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::tests::check_assist;

    #[test]
    fn add_return_type_with_no_previous_type() {
        check_assist(
            add_return_type,
            r#"
struct Foo<T>(T);
fn foo() {
    let x = 1usize + 1;
    $0Foo(x)
}
"#,
            r#"
struct Foo<T>(T);
fn foo() -> Foo<usize> {
    let x = 1usize + 1;
    Foo(x)
}
"#,
        );
    }

    #[test]
    fn add_return_type_ref_with_no_previous_type() {
        check_assist(
            add_return_type,
            r#"
fn foo<T>(t: &T) {
    $0t
}
"#,
            r#"
fn foo<T>(t: &T) -> &T {
    t
}
"#,
        );
    }

    #[test]
    fn add_return_type_ref_explicit_lifetime_with_no_previous_type() {
        check_assist(
            add_return_type,
            r#"
fn foo() {
    $0"Hello, world"
}
"#,
            // TODO: this won't compile because of a lack of a `'static` lifetime annotation
            r#"
fn foo() -> &str {
    "Hello, world"
}
"#,
        );
    }
}
