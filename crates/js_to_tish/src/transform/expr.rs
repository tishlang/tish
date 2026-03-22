//! Convert OXC expressions to Tish expressions.

use std::sync::Arc;

use oxc::ast::ast::Expression as OxcExpr;
use oxc::semantic::Semantic;
use tishlang_ast::{
    ArrayElement, ArrowBody, BinOp, CompoundOp, DestructPattern, Expr, Literal, LogicalAssignOp,
    MemberProp, ObjectProp, TypedParam,
};

use crate::error::{ConvertError, ConvertErrorKind};
use crate::span_util;

type Ctx<'a> = (&'a Semantic<'a>, &'a str);

/// Convert OXC expression to Tish expression.
pub fn convert_expr(expr: &OxcExpr<'_>, ctx: &Ctx<'_>) -> Result<Expr, ConvertError> {
    let span = span_util::oxc_span_to_tish(ctx.1, expr);
    match expr {
        OxcExpr::BooleanLiteral(b) => Ok(Expr::Literal {
            value: Literal::Bool(b.value),
            span,
        }),
        OxcExpr::NullLiteral(_) => Ok(Expr::Literal {
            value: Literal::Null,
            span,
        }),
        OxcExpr::NumericLiteral(n) => Ok(Expr::Literal {
            value: Literal::Number(n.value),
            span,
        }),
        OxcExpr::StringLiteral(s) => Ok(Expr::Literal {
            value: Literal::String(Arc::from(s.value.as_str())),
            span,
        }),
        OxcExpr::Identifier(id) => {
            let name = id.name.as_str();
            if name == "this" {
                return Err(ConvertError::new(ConvertErrorKind::Incompatible {
                    what: "this".into(),
                    reason: "Tish does not support this".into(),
                }));
            }
            if name == "arguments" {
                return Err(ConvertError::new(ConvertErrorKind::Incompatible {
                    what: "arguments".into(),
                    reason: "Tish does not support arguments object".into(),
                }));
            }
            Ok(Expr::Ident {
                name: Arc::from(name),
                span,
            })
        }
        OxcExpr::BinaryExpression(b) => {
            let left = Box::new(convert_expr(&b.left, ctx)?);
            let right = Box::new(convert_expr(&b.right, ctx)?);
            let op = convert_bin_op(&b.operator)?;
            Ok(Expr::Binary { left, op, right, span })
        }
        OxcExpr::UnaryExpression(u) => {
            let operand = Box::new(convert_expr(&u.argument, ctx)?);
            let op = convert_unary_op(&u.operator)?;
            Ok(Expr::Unary { op, operand, span })
        }
        OxcExpr::CallExpression(c) => {
            let callee = Box::new(convert_expr(&c.callee, ctx)?);
            let args = c
                .arguments
                .iter()
                .map(|a| convert_call_arg(a, ctx))
                .collect::<Result<Vec<_>, _>>()?;
            Ok(Expr::Call { callee, args, span })
        }
        OxcExpr::StaticMemberExpression(s) => {
            let object = Box::new(convert_expr(&s.object, ctx)?);
            Ok(Expr::Member {
                object,
                prop: MemberProp::Name(Arc::from(s.property.name.as_str())),
                optional: s.optional,
                span,
            })
        }
        OxcExpr::ComputedMemberExpression(c) => {
            let object = Box::new(convert_expr(&c.object, ctx)?);
            Ok(Expr::Member {
                object,
                prop: MemberProp::Expr(Box::new(convert_expr(&c.expression, ctx)?)),
                optional: c.optional,
                span,
            })
        }
        OxcExpr::ConditionalExpression(c) => {
            let cond = Box::new(convert_expr(&c.test, ctx)?);
            let then_branch = Box::new(convert_expr(&c.consequent, ctx)?);
            let else_branch = Box::new(convert_expr(&c.alternate, ctx)?);
            Ok(Expr::Conditional {
                cond,
                then_branch,
                else_branch,
                span,
            })
        }
        OxcExpr::LogicalExpression(l) => {
            if matches!(
                l.operator,
                oxc::ast::ast::LogicalOperator::Coalesce
            ) {
                Ok(Expr::NullishCoalesce {
                    left: Box::new(convert_expr(&l.left, ctx)?),
                    right: Box::new(convert_expr(&l.right, ctx)?),
                    span,
                })
            } else {
                let op = match l.operator {
                    oxc::ast::ast::LogicalOperator::And => BinOp::And,
                    oxc::ast::ast::LogicalOperator::Or => BinOp::Or,
                    _ => BinOp::Or,
                };
                Ok(Expr::Binary {
                    left: Box::new(convert_expr(&l.left, ctx)?),
                    op,
                    right: Box::new(convert_expr(&l.right, ctx)?),
                    span,
                })
            }
        }
        OxcExpr::AssignmentExpression(a) => convert_assignment(a, ctx, span),
        OxcExpr::ArrayExpression(arr) => {
            let elements = arr
                .elements
                .iter()
                .map(|e| convert_array_element(e, ctx))
                .collect::<Result<Vec<_>, _>>()?;
            Ok(Expr::Array { elements, span })
        }
        OxcExpr::ObjectExpression(obj) => {
            let props = obj
                .properties
                .iter()
                .map(|p| convert_object_prop(p, ctx))
                .collect::<Result<Vec<_>, _>>()?;
            Ok(Expr::Object { props, span })
        }
        OxcExpr::ArrowFunctionExpression(arrow) => {
            let params = convert_arrow_params(&arrow.params, ctx)?;
            let body = if arrow.expression {
                let e = arrow
                    .get_expression()
                    .ok_or_else(|| ConvertError::new(ConvertErrorKind::Incompatible {
                        what: "arrow expression body".into(),
                        reason: "expected expression".into(),
                    }))?;
                ArrowBody::Expr(Box::new(convert_expr(e, ctx)?))
            } else {
                let stmts = super::stmt::convert_statements(&arrow.body.statements, ctx.0, ctx.1)?;
                ArrowBody::Block(Box::new(tishlang_ast::Statement::Block {
                    statements: stmts,
                    span: span_util::oxc_span_to_tish(ctx.1, &*arrow.body),
                }))
            };
            Ok(Expr::ArrowFunction {
                params,
                body,
                span,
            })
        }
        OxcExpr::AwaitExpression(a) => Ok(Expr::Await {
            operand: Box::new(convert_expr(&a.argument, ctx)?),
            span,
        }),
        OxcExpr::TemplateLiteral(t) => {
            let mut quasis = Vec::new();
            let mut exprs = Vec::new();
            for q in &t.quasis {
                quasis.push(Arc::from(q.value.raw.as_str()));
            }
            for e in &t.expressions {
                exprs.push(convert_expr(e, ctx)?);
            }
            Ok(Expr::TemplateLiteral {
                quasis,
                exprs,
                span,
            })
        }
        OxcExpr::ChainExpression(c) => convert_chain_element(&c.expression, ctx, span),
        OxcExpr::UpdateExpression(u) => convert_update_expr(u, ctx, span),
        OxcExpr::SequenceExpression(s) => {
            // (a, b, c) -> convert last expr
            if let Some(last) = s.expressions.iter().last() {
                convert_expr(last, ctx)
            } else {
                Err(ConvertError::new(ConvertErrorKind::Incompatible {
                    what: "empty sequence".into(),
                    reason: "invalid".into(),
                }))
            }
        }
        OxcExpr::FunctionExpression(f) => {
            // Convert to ArrowFunction-like; Tish doesn't have function expressions per se
            // Use ArrowFunction with block body. FunctionBody is a struct with .statements.
            let params = convert_params(&f.params, ctx)?.0;
            let body = match &f.body {
                Some(fb) => {
                    let stmts = super::stmt::convert_statements(&fb.statements, ctx.0, ctx.1)?;
                    ArrowBody::Block(Box::new(tishlang_ast::Statement::Block {
                        statements: stmts,
                        span: span_util::oxc_span_to_tish(ctx.1, fb.as_ref()),
                    }))
                }
                None => {
                    return Err(ConvertError::new(ConvertErrorKind::Incompatible {
                        what: "function expression body".into(),
                        reason: "expected block".into(),
                    }))
                }
            };
            Ok(Expr::ArrowFunction {
                params,
                body,
                span,
            })
        }
        _ => Err(ConvertError::new(ConvertErrorKind::Unsupported {
            what: format!("expression: {:?}", std::mem::discriminant(expr)),
            hint: None,
        })),
    }
}

fn convert_chain_element(
    ce: &oxc::ast::ast::ChainElement<'_>,
    ctx: &Ctx<'_>,
    span: tishlang_ast::Span,
) -> Result<Expr, ConvertError> {
    match ce {
        oxc::ast::ast::ChainElement::StaticMemberExpression(m) => {
            let object = Box::new(convert_expr(&m.object, ctx)?);
            Ok(Expr::Member {
                object,
                prop: MemberProp::Name(Arc::from(m.property.name.as_str())),
                optional: m.optional,
                span,
            })
        }
        oxc::ast::ast::ChainElement::ComputedMemberExpression(m) => {
            let object = Box::new(convert_expr(&m.object, ctx)?);
            Ok(Expr::Member {
                object,
                prop: MemberProp::Expr(Box::new(convert_expr(&m.expression, ctx)?)),
                optional: m.optional,
                span,
            })
        }
        oxc::ast::ast::ChainElement::CallExpression(call) => {
            let callee = Box::new(convert_expr(&call.callee, ctx)?);
            let args = call
                .arguments
                .iter()
                .map(|a| convert_call_arg(a, ctx))
                .collect::<Result<Vec<_>, _>>()?;
            Ok(Expr::Call { callee, args, span })
        }
        _ => Err(ConvertError::new(ConvertErrorKind::Unsupported {
            what: "chain expression element (e.g. TSNonNull, PrivateField)".into(),
            hint: None,
        })),
    }
}

fn convert_call_arg(
    arg: &oxc::ast::ast::Argument<'_>,
    ctx: &Ctx<'_>,
) -> Result<tishlang_ast::CallArg, ConvertError> {
    if arg.is_spread() {
        if let oxc::ast::ast::Argument::SpreadElement(s) = arg {
            Ok(tishlang_ast::CallArg::Spread(convert_expr(&s.argument, ctx)?))
        } else {
            unreachable!()
        }
    } else if let Some(e) = arg.as_expression() {
        Ok(tishlang_ast::CallArg::Expr(convert_expr(e, ctx)?))
    } else {
        Err(ConvertError::new(ConvertErrorKind::Unsupported {
            what: "call argument".into(),
            hint: None,
        }))
    }
}

fn convert_array_element(
    el: &oxc::ast::ast::ArrayExpressionElement<'_>,
    ctx: &Ctx<'_>,
) -> Result<ArrayElement, ConvertError> {
    match el {
        oxc::ast::ast::ArrayExpressionElement::SpreadElement(s) => {
            Ok(ArrayElement::Spread(convert_expr(&s.argument, ctx)?))
        }
        oxc::ast::ast::ArrayExpressionElement::Elision(_) => Ok(ArrayElement::Expr(Expr::Literal {
            value: Literal::Null,
            span: span_util::stub_span(),
        })),
        _ => {
            if let Some(e) = el.as_expression() {
                Ok(ArrayElement::Expr(convert_expr(e, ctx)?))
            } else {
                Err(ConvertError::new(ConvertErrorKind::Unsupported {
                    what: "array element".into(),
                    hint: None,
                }))
            }
        }
    }
}

fn convert_object_prop(
    p: &oxc::ast::ast::ObjectPropertyKind<'_>,
    ctx: &Ctx<'_>,
) -> Result<ObjectProp, ConvertError> {
    match p {
        oxc::ast::ast::ObjectPropertyKind::SpreadProperty(s) => {
            Ok(ObjectProp::Spread(convert_expr(&s.argument, ctx)?))
        }
        oxc::ast::ast::ObjectPropertyKind::ObjectProperty(prop) => {
            let key = prop.key.name().map(|n| n.to_string()).unwrap_or_else(|| {
                if let oxc::ast::ast::PropertyKey::Identifier(id) = &prop.key {
                    id.name.to_string()
                } else {
                    "key".to_string()
                }
            });
            let value = convert_expr(&prop.value, ctx)?;
            Ok(ObjectProp::KeyValue(Arc::from(key.as_str()), value))
        }
    }
}

fn convert_assignment(
    a: &oxc::ast::ast::AssignmentExpression<'_>,
    ctx: &Ctx<'_>,
    span: tishlang_ast::Span,
) -> Result<Expr, ConvertError> {
    let (left, right) = (&a.left, &a.right);
    if let Some(oxc::ast::ast::SimpleAssignmentTarget::AssignmentTargetIdentifier(id)) =
        left.as_simple_assignment_target()
    {
        let name = id.name.as_str();
        let value = Box::new(convert_expr(right, ctx)?);
        return match &a.operator {
                oxc::ast::ast::AssignmentOperator::Assign => {
                    Ok(Expr::Assign {
                        name: Arc::from(name),
                        value,
                        span,
                    })
                }
                oxc::ast::ast::AssignmentOperator::Addition => {
                    Ok(Expr::CompoundAssign {
                        name: Arc::from(name),
                        op: CompoundOp::Add,
                        value,
                        span,
                    })
                }
                oxc::ast::ast::AssignmentOperator::Subtraction => {
                    Ok(Expr::CompoundAssign {
                        name: Arc::from(name),
                        op: CompoundOp::Sub,
                        value,
                        span,
                    })
                }
                oxc::ast::ast::AssignmentOperator::Multiplication => {
                    Ok(Expr::CompoundAssign {
                        name: Arc::from(name),
                        op: CompoundOp::Mul,
                        value,
                        span,
                    })
                }
                oxc::ast::ast::AssignmentOperator::Division => {
                    Ok(Expr::CompoundAssign {
                        name: Arc::from(name),
                        op: CompoundOp::Div,
                        value,
                        span,
                    })
                }
                oxc::ast::ast::AssignmentOperator::Remainder => {
                    Ok(Expr::CompoundAssign {
                        name: Arc::from(name),
                        op: CompoundOp::Mod,
                        value,
                        span,
                    })
                }
                oxc::ast::ast::AssignmentOperator::LogicalAnd => {
                    Ok(Expr::LogicalAssign {
                        name: Arc::from(name),
                        op: LogicalAssignOp::AndAnd,
                        value,
                        span,
                    })
                }
                oxc::ast::ast::AssignmentOperator::LogicalOr => {
                    Ok(Expr::LogicalAssign {
                        name: Arc::from(name),
                        op: LogicalAssignOp::OrOr,
                        value,
                        span,
                    })
                }
                oxc::ast::ast::AssignmentOperator::LogicalNullish => {
                    Ok(Expr::LogicalAssign {
                        name: Arc::from(name),
                        op: LogicalAssignOp::Nullish,
                        value,
                        span,
                    })
                }
            _ => Err(ConvertError::new(ConvertErrorKind::Unsupported {
                what: "assignment operator".into(),
                hint: None,
            })),
        };
    }
    Err(ConvertError::new(ConvertErrorKind::Unsupported {
        what: "complex assignment target".into(),
        hint: None,
    }))
}

fn convert_update_expr(
    u: &oxc::ast::ast::UpdateExpression<'_>,
    _ctx: &Ctx<'_>,
    span: tishlang_ast::Span,
) -> Result<Expr, ConvertError> {
    let name: Arc<str> = match &u.argument {
        oxc::ast::ast::SimpleAssignmentTarget::AssignmentTargetIdentifier(id) => {
            Arc::from(id.name.as_str())
        }
        _ => {
            return Err(ConvertError::new(ConvertErrorKind::Unsupported {
                what: "update expression on non-identifier".into(),
                hint: None,
            }))
        }
    };
    Ok(match (u.operator, u.prefix) {
        (oxc::ast::ast::UpdateOperator::Increment, true) => {
            Expr::PrefixInc { name, span }
        }
        (oxc::ast::ast::UpdateOperator::Increment, false) => {
            Expr::PostfixInc { name, span }
        }
        (oxc::ast::ast::UpdateOperator::Decrement, true) => {
            Expr::PrefixDec { name, span }
        }
        (oxc::ast::ast::UpdateOperator::Decrement, false) => {
            Expr::PostfixDec { name, span }
        }
    })
}

fn convert_bin_op(
    op: &oxc::ast::ast::BinaryOperator,
) -> Result<BinOp, ConvertError> {
    Ok(match op {
        oxc::ast::ast::BinaryOperator::Equality => BinOp::Eq,
        oxc::ast::ast::BinaryOperator::Inequality => BinOp::Ne,
        oxc::ast::ast::BinaryOperator::StrictEquality => BinOp::StrictEq,
        oxc::ast::ast::BinaryOperator::StrictInequality => BinOp::StrictNe,
        oxc::ast::ast::BinaryOperator::LessThan => BinOp::Lt,
        oxc::ast::ast::BinaryOperator::LessEqualThan => BinOp::Le,
        oxc::ast::ast::BinaryOperator::GreaterThan => BinOp::Gt,
        oxc::ast::ast::BinaryOperator::GreaterEqualThan => BinOp::Ge,
        oxc::ast::ast::BinaryOperator::Addition => BinOp::Add,
        oxc::ast::ast::BinaryOperator::Subtraction => BinOp::Sub,
        oxc::ast::ast::BinaryOperator::Multiplication => BinOp::Mul,
        oxc::ast::ast::BinaryOperator::Division => BinOp::Div,
        oxc::ast::ast::BinaryOperator::Remainder => BinOp::Mod,
        oxc::ast::ast::BinaryOperator::Exponential => BinOp::Pow,
        oxc::ast::ast::BinaryOperator::BitwiseAnd => BinOp::BitAnd,
        oxc::ast::ast::BinaryOperator::BitwiseOR => BinOp::BitOr,
        oxc::ast::ast::BinaryOperator::BitwiseXOR => BinOp::BitXor,
        oxc::ast::ast::BinaryOperator::ShiftLeft => BinOp::Shl,
        oxc::ast::ast::BinaryOperator::ShiftRight => BinOp::Shr,
        _ => {
            return Err(ConvertError::new(ConvertErrorKind::Unsupported {
                what: format!("binary operator: {op:?}"),
                hint: None,
            }))
        }
    })
}

fn convert_unary_op(
    op: &oxc::ast::ast::UnaryOperator,
) -> Result<tishlang_ast::UnaryOp, ConvertError> {
    Ok(match op {
        oxc::ast::ast::UnaryOperator::LogicalNot => tishlang_ast::UnaryOp::Not,
        oxc::ast::ast::UnaryOperator::UnaryNegation => tishlang_ast::UnaryOp::Neg,
        oxc::ast::ast::UnaryOperator::UnaryPlus => tishlang_ast::UnaryOp::Pos,
        oxc::ast::ast::UnaryOperator::BitwiseNot => tishlang_ast::UnaryOp::BitNot,
        oxc::ast::ast::UnaryOperator::Void => tishlang_ast::UnaryOp::Void,
        _ => {
            return Err(ConvertError::new(ConvertErrorKind::Unsupported {
                what: format!("unary operator: {op:?}"),
                hint: None,
            }))
        }
    })
}

/// Convert function/arrow params to TypedParam list.
pub fn convert_params(
    params: &oxc::ast::ast::FormalParameters<'_>,
    ctx: &Ctx<'_>,
) -> Result<(Vec<TypedParam>, Option<TypedParam>), ConvertError> {
    let mut typed_params = Vec::new();
    let mut rest_param = None;
    for (i, p) in params.items.iter().enumerate() {
        if params.rest.is_some() && i == params.items.len() - 1 {
            if let Some(rest) = &params.rest {
                let rest_name = match &rest.rest.argument {
                    oxc::ast::ast::BindingPattern::BindingIdentifier(b) => b.name.as_str(),
                    _ => {
                        return Err(ConvertError::new(ConvertErrorKind::Unsupported {
                            what: "rest param with non-identifier".into(),
                            hint: None,
                        }))
                    }
                };
                rest_param = Some(TypedParam {
                    name: Arc::from(rest_name),
                    type_ann: None,
                    default: None,
                });
            }
            break;
        }
        // params.items contains FormalParameter structs (not enum)
        let fp = p;
        {
            let name = match &fp.pattern {
                    oxc::ast::ast::BindingPattern::BindingIdentifier(b) => b.name.as_str(),
                    _ => {
                        return Err(ConvertError::new(ConvertErrorKind::Unsupported {
                            what: "destructuring in params".into(),
                            hint: None,
                        }))
                    }
                };
                let default = fp
                    .initializer
                    .as_ref()
                    .map(|e| convert_expr(e, ctx))
                    .transpose()?;
                typed_params.push(TypedParam {
                    name: Arc::from(name),
                    type_ann: None,
                    default,
                });
        }
    }
    if rest_param.is_none() {
        if let Some(rest) = &params.rest {
            let rest_name = match &rest.rest.argument {
                oxc::ast::ast::BindingPattern::BindingIdentifier(b) => b.name.as_str(),
                _ => {
                    return Err(ConvertError::new(ConvertErrorKind::Unsupported {
                        what: "rest param with non-identifier".into(),
                        hint: None,
                    }));
                }
            };
            rest_param = Some(TypedParam {
                name: Arc::from(rest_name),
                type_ann: None,
                default: None,
            });
        }
    }
    Ok((typed_params, rest_param))
}

fn convert_arrow_params(
    params: &oxc::ast::ast::FormalParameters<'_>,
    ctx: &Ctx<'_>,
) -> Result<Vec<TypedParam>, ConvertError> {
    let (mut ps, rest) = convert_params(params, ctx)?;
    if let Some(r) = rest {
        ps.push(r);
    }
    Ok(ps)
}

/// Convert binding pattern to Tish DestructPattern.
pub fn convert_destruct_pattern(
    _pattern: &oxc::ast::ast::BindingPattern<'_>,
) -> Result<DestructPattern, ConvertError> {
    Err(ConvertError::new(ConvertErrorKind::Unsupported {
        what: "destructuring in variable declaration".into(),
        hint: Some("use simple identifier for now".into()),
    }))
}
