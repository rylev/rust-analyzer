//! This module contains functions to generate default trait impl function bodies where possible.

use syntax::{
    ast::{self, edit::AstNodeEdit, make, AstNode, NameOwner},
    ted,
};

/// Generate custom trait bodies where possible.
///
/// Returns `Option` so that we can use `?` rather than `if let Some`. Returning
/// `None` means that generating a custom trait body failed, and the body will remain
/// as `todo!` instead.
pub(crate) fn gen_trait_fn_body(
    func: &ast::Fn,
    trait_path: &ast::Path,
    adt: &ast::Adt,
) -> Option<()> {
    match trait_path.segment()?.name_ref()?.text().as_str() {
        "Clone" => gen_clone_impl(adt, func),
        "Debug" => gen_debug_impl(adt, func),
        "Default" => gen_default_impl(adt, func),
        "Hash" => gen_hash_impl(adt, func),
        _ => None,
    }
}

/// Generate a `Clone` impl based on the fields and members of the target type.
fn gen_clone_impl(adt: &ast::Adt, func: &ast::Fn) -> Option<()> {
    fn gen_clone_call(target: ast::Expr) -> ast::Expr {
        let method = make::name_ref("clone");
        make::expr_method_call(target, method, make::arg_list(None))
    }
    let expr = match adt {
        // `Clone` cannot be derived for unions, so no default impl can be provided.
        ast::Adt::Union(_) => return None,
        ast::Adt::Enum(enum_) => {
            let list = enum_.variant_list()?;
            let mut arms = vec![];
            for variant in list.variants() {
                let name = variant.name()?;
                let left = make::ext::ident_path("Self");
                let right = make::ext::ident_path(&format!("{}", name));
                let variant_name = make::path_concat(left, right);

                match variant.field_list() {
                    // => match self { Self::Name { x } => Self::Name { x: x.clone() } }
                    Some(ast::FieldList::RecordFieldList(list)) => {
                        let mut pats = vec![];
                        let mut fields = vec![];
                        for field in list.fields() {
                            let field_name = field.name()?;
                            let pat = make::ident_pat(false, false, field_name.clone());
                            pats.push(pat.into());

                            let path = make::ext::ident_path(&field_name.to_string());
                            let method_call = gen_clone_call(make::expr_path(path));
                            let name_ref = make::name_ref(&field_name.to_string());
                            let field = make::record_expr_field(name_ref, Some(method_call));
                            fields.push(field);
                        }
                        let pat = make::record_pat(variant_name.clone(), pats.into_iter());
                        let fields = make::record_expr_field_list(fields);
                        let record_expr = make::record_expr(variant_name, fields).into();
                        arms.push(make::match_arm(Some(pat.into()), None, record_expr));
                    }

                    // => match self { Self::Name(arg1) => Self::Name(arg1.clone()) }
                    Some(ast::FieldList::TupleFieldList(list)) => {
                        let mut pats = vec![];
                        let mut fields = vec![];
                        for (i, _) in list.fields().enumerate() {
                            let field_name = format!("arg{}", i);
                            let pat = make::ident_pat(false, false, make::name(&field_name));
                            pats.push(pat.into());

                            let f_path = make::expr_path(make::ext::ident_path(&field_name));
                            fields.push(gen_clone_call(f_path));
                        }
                        let pat = make::tuple_struct_pat(variant_name.clone(), pats.into_iter());
                        let struct_name = make::expr_path(variant_name);
                        let tuple_expr = make::expr_call(struct_name, make::arg_list(fields));
                        arms.push(make::match_arm(Some(pat.into()), None, tuple_expr));
                    }

                    // => match self { Self::Name => Self::Name }
                    None => {
                        let pattern = make::path_pat(variant_name.clone());
                        let variant_expr = make::expr_path(variant_name);
                        arms.push(make::match_arm(Some(pattern.into()), None, variant_expr));
                    }
                }
            }

            let match_target = make::expr_path(make::ext::ident_path("self"));
            let list = make::match_arm_list(arms).indent(ast::edit::IndentLevel(1));
            make::expr_match(match_target, list)
        }
        ast::Adt::Struct(strukt) => {
            match strukt.field_list() {
                // => Self { name: self.name.clone() }
                Some(ast::FieldList::RecordFieldList(field_list)) => {
                    let mut fields = vec![];
                    for field in field_list.fields() {
                        let base = make::expr_path(make::ext::ident_path("self"));
                        let target = make::expr_field(base, &field.name()?.to_string());
                        let method_call = gen_clone_call(target);
                        let name_ref = make::name_ref(&field.name()?.to_string());
                        let field = make::record_expr_field(name_ref, Some(method_call));
                        fields.push(field);
                    }
                    let struct_name = make::ext::ident_path("Self");
                    let fields = make::record_expr_field_list(fields);
                    make::record_expr(struct_name, fields).into()
                }
                // => Self(self.0.clone(), self.1.clone())
                Some(ast::FieldList::TupleFieldList(field_list)) => {
                    let mut fields = vec![];
                    for (i, _) in field_list.fields().enumerate() {
                        let f_path = make::expr_path(make::ext::ident_path("self"));
                        let target = make::expr_field(f_path, &format!("{}", i)).into();
                        fields.push(gen_clone_call(target));
                    }
                    let struct_name = make::expr_path(make::ext::ident_path("Self"));
                    make::expr_call(struct_name, make::arg_list(fields))
                }
                // => Self { }
                None => {
                    let struct_name = make::ext::ident_path("Self");
                    let fields = make::record_expr_field_list(None);
                    make::record_expr(struct_name, fields).into()
                }
            }
        }
    };
    let body = make::block_expr(None, Some(expr)).indent(ast::edit::IndentLevel(1));
    ted::replace(func.body()?.syntax(), body.clone_for_update().syntax());
    Some(())
}

/// Generate a `Debug` impl based on the fields and members of the target type.
fn gen_debug_impl(adt: &ast::Adt, func: &ast::Fn) -> Option<()> {
    let annotated_name = adt.name()?;
    match adt {
        // `Debug` cannot be derived for unions, so no default impl can be provided.
        ast::Adt::Union(_) => None,

        // => match self { Self::Variant => write!(f, "Variant") }
        ast::Adt::Enum(enum_) => {
            let list = enum_.variant_list()?;
            let mut arms = vec![];
            for variant in list.variants() {
                let name = variant.name()?;
                let left = make::ext::ident_path("Self");
                let right = make::ext::ident_path(&format!("{}", name));
                let variant_name = make::path_pat(make::path_concat(left, right));

                let target = make::expr_path(make::ext::ident_path("f").into());
                let fmt_string = make::expr_literal(&(format!("\"{}\"", name))).into();
                let args = make::arg_list(vec![target, fmt_string]);
                let macro_name = make::expr_path(make::ext::ident_path("write"));
                let macro_call = make::expr_macro_call(macro_name, args);

                arms.push(make::match_arm(Some(variant_name.into()), None, macro_call.into()));
            }

            let match_target = make::expr_path(make::ext::ident_path("self"));
            let list = make::match_arm_list(arms).indent(ast::edit::IndentLevel(1));
            let match_expr = make::expr_match(match_target, list);

            let body = make::block_expr(None, Some(match_expr));
            let body = body.indent(ast::edit::IndentLevel(1));
            ted::replace(func.body()?.syntax(), body.clone_for_update().syntax());
            Some(())
        }

        ast::Adt::Struct(strukt) => {
            let name = format!("\"{}\"", annotated_name);
            let args = make::arg_list(Some(make::expr_literal(&name).into()));
            let target = make::expr_path(make::ext::ident_path("f"));

            let expr = match strukt.field_list() {
                // => f.debug_struct("Name").finish()
                None => make::expr_method_call(target, make::name_ref("debug_struct"), args),

                // => f.debug_struct("Name").field("foo", &self.foo).finish()
                Some(ast::FieldList::RecordFieldList(field_list)) => {
                    let method = make::name_ref("debug_struct");
                    let mut expr = make::expr_method_call(target, method, args);
                    for field in field_list.fields() {
                        let name = field.name()?;
                        let f_name = make::expr_literal(&(format!("\"{}\"", name))).into();
                        let f_path = make::expr_path(make::ext::ident_path("self"));
                        let f_path = make::expr_ref(f_path, false);
                        let f_path = make::expr_field(f_path, &format!("{}", name)).into();
                        let args = make::arg_list(vec![f_name, f_path]);
                        expr = make::expr_method_call(expr, make::name_ref("field"), args);
                    }
                    expr
                }

                // => f.debug_tuple("Name").field(self.0).finish()
                Some(ast::FieldList::TupleFieldList(field_list)) => {
                    let method = make::name_ref("debug_tuple");
                    let mut expr = make::expr_method_call(target, method, args);
                    for (i, _) in field_list.fields().enumerate() {
                        let f_path = make::expr_path(make::ext::ident_path("self"));
                        let f_path = make::expr_ref(f_path, false);
                        let f_path = make::expr_field(f_path, &format!("{}", i)).into();
                        let method = make::name_ref("field");
                        expr = make::expr_method_call(expr, method, make::arg_list(Some(f_path)));
                    }
                    expr
                }
            };

            let method = make::name_ref("finish");
            let expr = make::expr_method_call(expr, method, make::arg_list(None));
            let body = make::block_expr(None, Some(expr)).indent(ast::edit::IndentLevel(1));
            ted::replace(func.body()?.syntax(), body.clone_for_update().syntax());
            Some(())
        }
    }
}

/// Generate a `Debug` impl based on the fields and members of the target type.
fn gen_default_impl(adt: &ast::Adt, func: &ast::Fn) -> Option<()> {
    fn gen_default_call() -> ast::Expr {
        let trait_name = make::ext::ident_path("Default");
        let method_name = make::ext::ident_path("default");
        let fn_name = make::expr_path(make::path_concat(trait_name, method_name));
        make::expr_call(fn_name, make::arg_list(None))
    }
    match adt {
        // `Debug` cannot be derived for unions, so no default impl can be provided.
        ast::Adt::Union(_) => None,
        // Deriving `Debug` for enums is not stable yet.
        ast::Adt::Enum(_) => None,
        ast::Adt::Struct(strukt) => {
            let expr = match strukt.field_list() {
                Some(ast::FieldList::RecordFieldList(field_list)) => {
                    let mut fields = vec![];
                    for field in field_list.fields() {
                        let method_call = gen_default_call();
                        let name_ref = make::name_ref(&field.name()?.to_string());
                        let field = make::record_expr_field(name_ref, Some(method_call));
                        fields.push(field);
                    }
                    let struct_name = make::ext::ident_path("Self");
                    let fields = make::record_expr_field_list(fields);
                    make::record_expr(struct_name, fields).into()
                }
                Some(ast::FieldList::TupleFieldList(field_list)) => {
                    let struct_name = make::expr_path(make::ext::ident_path("Self"));
                    let fields = field_list.fields().map(|_| gen_default_call());
                    make::expr_call(struct_name, make::arg_list(fields))
                }
                None => {
                    let struct_name = make::ext::ident_path("Self");
                    let fields = make::record_expr_field_list(None);
                    make::record_expr(struct_name, fields).into()
                }
            };
            let body = make::block_expr(None, Some(expr)).indent(ast::edit::IndentLevel(1));
            ted::replace(func.body()?.syntax(), body.clone_for_update().syntax());
            Some(())
        }
    }
}

/// Generate a `Hash` impl based on the fields and members of the target type.
fn gen_hash_impl(adt: &ast::Adt, func: &ast::Fn) -> Option<()> {
    fn gen_hash_call(target: ast::Expr) -> ast::Stmt {
        let method = make::name_ref("hash");
        let arg = make::expr_path(make::ext::ident_path("state"));
        let expr = make::expr_method_call(target, method, make::arg_list(Some(arg)));
        let stmt = make::expr_stmt(expr);
        stmt.into()
    }

    let body = match adt {
        // `Hash` cannot be derived for unions, so no default impl can be provided.
        ast::Adt::Union(_) => return None,

        // => std::mem::discriminant(self).hash(state);
        ast::Adt::Enum(_) => {
            let root = make::ext::ident_path("core");
            let submodule = make::ext::ident_path("mem");
            let fn_name = make::ext::ident_path("discriminant");
            let fn_name = make::path_concat(submodule, fn_name);
            let fn_name = make::expr_path(make::path_concat(root, fn_name));

            let arg = make::expr_path(make::ext::ident_path("self"));
            let fn_call = make::expr_call(fn_name, make::arg_list(Some(arg)));
            let stmt = gen_hash_call(fn_call);

            make::block_expr(Some(stmt), None).indent(ast::edit::IndentLevel(1))
        }
        ast::Adt::Struct(strukt) => match strukt.field_list() {
            // => self.<field>.hash(state);
            Some(ast::FieldList::RecordFieldList(field_list)) => {
                let mut stmts = vec![];
                for field in field_list.fields() {
                    let base = make::expr_path(make::ext::ident_path("self"));
                    let target = make::expr_field(base, &field.name()?.to_string());
                    stmts.push(gen_hash_call(target));
                }
                make::block_expr(stmts, None).indent(ast::edit::IndentLevel(1))
            }

            // => self.<field_index>.hash(state);
            Some(ast::FieldList::TupleFieldList(field_list)) => {
                let mut stmts = vec![];
                for (i, _) in field_list.fields().enumerate() {
                    let base = make::expr_path(make::ext::ident_path("self"));
                    let target = make::expr_field(base, &format!("{}", i));
                    stmts.push(gen_hash_call(target));
                }
                make::block_expr(stmts, None).indent(ast::edit::IndentLevel(1))
            }

            // No fields in the body means there's nothing to hash.
            None => return None,
        },
    };

    ted::replace(func.body()?.syntax(), body.clone_for_update().syntax());
    Some(())
}
