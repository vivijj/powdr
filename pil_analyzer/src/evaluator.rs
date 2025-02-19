use std::{collections::HashMap, fmt::Display, iter::repeat, rc::Rc};

use ast::{
    analyzed::{Expression, FunctionValueDefinition, Reference, Symbol},
    evaluate_binary_operation, evaluate_unary_operation,
    parsed::{display::quote, FunctionCall, MatchArm, MatchPattern},
};
use itertools::Itertools;
use number::FieldElement;

/// Evaluates an expression given a hash map of definitions.
pub fn evaluate_expression<'a, T: FieldElement>(
    expr: &'a Expression<T>,
    definitions: &'a HashMap<String, (Symbol, Option<FunctionValueDefinition<T>>)>,
) -> Result<Value<'a, T, NoCustom>, EvalError> {
    evaluate(expr, &Definitions(definitions))
}

pub fn evaluate<'a, T: FieldElement, C: Custom>(
    expr: &'a Expression<T>,
    symbols: &impl SymbolLookup<'a, T, C>,
) -> Result<Value<'a, T, C>, EvalError> {
    internal::evaluate(expr, &[], symbols)
}

// TODO this function should be removed in the future, or at least it should
// check that `expr` actually evaluates to a Closure.
pub fn evaluate_function_call<'a, T: FieldElement, C: Custom>(
    expr: &'a Expression<T>,
    arguments: Vec<T>,
    symbols: &impl SymbolLookup<'a, T, C>,
) -> Result<Value<'a, T, C>, EvalError> {
    internal::evaluate(
        expr,
        &arguments
            .into_iter()
            .map(|x| Rc::new(Value::Number(x)))
            .collect::<Vec<_>>(),
        symbols,
    )
}

/// Evaluation errors.
/// TODO Most of these errors should be converted to panics as soon as we have a proper type checker.
#[derive(Debug)]
pub enum EvalError {
    /// Type error, for example non-number used as array index.
    TypeError(String),
    /// Fundamentally unsupported operation (regardless of type), e.g. access to public variables.
    Unsupported(String),
    /// Array index access out of bounds.
    OutOfBounds(String),
    /// Unable to match pattern. TODO As soon as we have "Option", patterns should be exhaustive
    /// This error occurs quite often and thus should not require allocation.
    NoMatch(),
    /// Reference to an undefined symbol
    SymbolNotFound(String),
    /// Data not (yet) available
    DataNotAvailable,
}

#[derive(Clone, PartialEq)]
pub enum Value<'a, T, C> {
    Number(T),
    String(String),
    Tuple(Vec<Self>),
    Array(Vec<Self>),
    Closure(Closure<'a, T, C>),
    Custom(C),
}

// TODO somehow, implementing TryFrom or TryInto did not work.

impl<'a, T: FieldElement, C: Custom> Value<'a, T, C> {
    pub fn try_to_number(self) -> Result<T, EvalError> {
        match self {
            Value::Number(x) => Ok(x),
            v => Err(EvalError::TypeError(format!("Expected number but got {v}"))),
        }
    }
}

pub trait Custom: Display + Clone + PartialEq {}

impl<'a, T: Display, C: Custom> Display for Value<'a, T, C> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Value::Number(x) => write!(f, "{x}"),
            Value::String(s) => write!(f, "{}", quote(s)),
            Value::Tuple(items) => write!(f, "({})", items.iter().format(", ")),
            Value::Array(elements) => write!(f, "[{}]", elements.iter().format(", ")),
            Value::Closure(closure) => write!(f, "{closure}"),
            Value::Custom(c) => write!(f, "{c}"),
        }
    }
}

#[derive(Clone, PartialEq)]
pub enum NoCustom {}

impl Custom for NoCustom {}

impl Display for NoCustom {
    fn fmt(&self, _f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        unreachable!()
    }
}

#[derive(Clone)]
pub struct Closure<'a, T, C> {
    // TODO we could also store the names of the parameters (for printing)
    // In order to do this, we would need to add a parameter name to Mapping, which might be a good idea anyway.
    pub parameter_count: usize,
    pub body: &'a Expression<T>,
    pub environment: Vec<Rc<Value<'a, T, C>>>,
}

impl<'a, T, C> PartialEq for Closure<'a, T, C> {
    fn eq(&self, _other: &Self) -> bool {
        // Eq is used for pattern matching.
        // In the future, we should introduce a proper pattern type.
        panic!("Tried to compare closures.");
    }
}

impl<'a, T: Display, C> Display for Closure<'a, T, C> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "|{}| {}",
            repeat('_').take(self.parameter_count).format(", "),
            self.body
        )
    }
}

impl<'a, T, C> From<Closure<'a, T, C>> for Value<'a, T, C> {
    fn from(value: Closure<'a, T, C>) -> Self {
        Value::Closure(value)
    }
}

pub struct Definitions<'a, T>(
    pub &'a HashMap<String, (Symbol, Option<FunctionValueDefinition<T>>)>,
);

impl<'a, T: FieldElement> SymbolLookup<'a, T, NoCustom> for Definitions<'a, T> {
    fn lookup(&self, name: &'a str) -> Result<Value<'a, T, NoCustom>, EvalError> {
        Ok(match self.0.get(&name.to_string()) {
            Some((_, value)) => match value {
                Some(FunctionValueDefinition::Expression(value)) => evaluate(value, self)?,
                Some(FunctionValueDefinition::Mapping(body)) => (Closure {
                    parameter_count: 1,
                    body,
                    environment: vec![],
                })
                .into(),
                _ => Err(EvalError::Unsupported(
                    "Cannot evaluate arrays and queries.".to_string(),
                ))?,
            },
            _ => Err(EvalError::SymbolNotFound(format!(
                "Symbol {name} not found."
            )))?,
        })
    }
    fn eval_function_application(
        &self,
        _function: NoCustom,
        _arguments: &[Rc<Value<'a, T, NoCustom>>],
    ) -> Result<Value<'a, T, NoCustom>, EvalError> {
        unreachable!()
    }
}

impl<'a, T: FieldElement> From<&'a HashMap<String, (Symbol, Option<FunctionValueDefinition<T>>)>>
    for Definitions<'a, T>
{
    fn from(value: &'a HashMap<String, (Symbol, Option<FunctionValueDefinition<T>>)>) -> Self {
        Definitions(value)
    }
}

pub trait SymbolLookup<'a, T, C> {
    fn lookup(&self, name: &'a str) -> Result<Value<'a, T, C>, EvalError>;
    fn lookup_public_reference(&self, name: &'a str) -> Result<Value<'a, T, C>, EvalError> {
        Err(EvalError::Unsupported(format!(
            "Cannot evaluate public reference: {name}"
        )))
    }
    fn eval_function_application(
        &self,
        function: C,
        arguments: &[Rc<Value<'a, T, C>>],
    ) -> Result<Value<'a, T, C>, EvalError>;
}

mod internal {
    use super::*;

    pub fn evaluate<'a, T: FieldElement, C: Custom>(
        expr: &'a Expression<T>,
        locals: &[Rc<Value<'a, T, C>>],
        symbols: &impl SymbolLookup<'a, T, C>,
    ) -> Result<Value<'a, T, C>, EvalError> {
        Ok(match expr {
            Expression::Reference(reference) => evaluate_reference(reference, locals, symbols)?,
            Expression::PublicReference(name) => symbols.lookup_public_reference(name)?,
            Expression::Number(n) => Value::Number(*n),
            Expression::String(s) => Value::String(s.clone()),
            Expression::Tuple(items) => Value::Tuple(
                items
                    .iter()
                    .map(|e| evaluate(e, locals, symbols))
                    .collect::<Result<_, _>>()?,
            ),
            Expression::ArrayLiteral(elements) => Value::Array(
                elements
                    .items
                    .iter()
                    .map(|e| evaluate(e, locals, symbols))
                    .collect::<Result<_, _>>()?,
            ),
            Expression::BinaryOperation(left, op, right) => {
                Value::Number(evaluate_binary_operation(
                    evaluate(left, locals, symbols)?.try_to_number()?,
                    *op,
                    evaluate(right, locals, symbols)?.try_to_number()?,
                ))
            }
            Expression::UnaryOperation(op, expr) => Value::Number(evaluate_unary_operation(
                *op,
                evaluate(expr, locals, symbols)?.try_to_number()?,
            )),
            Expression::LambdaExpression(lambda) => {
                // TODO only copy the part of the environment that is actually referenced?
                (Closure {
                    parameter_count: lambda.params.len(),
                    body: lambda.body.as_ref(),
                    environment: locals.to_vec(),
                })
                .into()
            }
            Expression::IndexAccess(index_access) => {
                match evaluate(&index_access.array, locals, symbols)? {
                    Value::Array(elements) => {
                        let index =
                            evaluate(&index_access.index, locals, symbols)?.try_to_number()?;
                        if index.to_integer() >= (elements.len() as u64).into() {
                            Err(EvalError::OutOfBounds(format!(
                                "Index access out of bounds: Tried to access element {index} of array of size {}.",
                                elements.len()
                            )))?;
                        }
                        elements
                            .into_iter()
                            .nth(index.to_degree() as usize)
                            .unwrap()
                    }
                    e => Err(EvalError::TypeError(format!("Expected array, but got {e}")))?,
                }
            }
            Expression::FunctionCall(FunctionCall { id, arguments }) => {
                let function = evaluate_reference(id, locals, symbols)?;
                let arguments = arguments
                    .iter()
                    .map(|a| evaluate(a, locals, symbols).map(Rc::new))
                    .collect::<Result<Vec<_>, _>>()?;
                match function {
                    Value::Closure(Closure {
                        parameter_count,
                        body,
                        environment,
                    }) => {
                        assert_eq!(parameter_count, arguments.len());

                        let local_vars =
                            arguments.into_iter().chain(environment).collect::<Vec<_>>();

                        evaluate(body, &local_vars, symbols)?
                    }
                    Value::Custom(value) => symbols.eval_function_application(value, &arguments)?,
                    e => Err(EvalError::TypeError(format!(
                        "Expected function but got {e}"
                    )))?,
                }
            }
            Expression::MatchExpression(scrutinee, arms) => {
                let v = evaluate(scrutinee, locals, symbols)?;
                let body = arms
                    .iter()
                    .find_map(|MatchArm { pattern, value }| match pattern {
                        MatchPattern::Pattern(p) => {
                            // TODO this uses PartialEq. As soon as we have proper match patterns
                            // instead of value, we can remove the PartialEq requirement on Value.
                            (evaluate(p, locals, symbols).unwrap() == v).then_some(value)
                        }
                        MatchPattern::CatchAll => Some(value),
                    })
                    .ok_or_else(EvalError::NoMatch)?;
                evaluate(body, locals, symbols)?
            }
            Expression::FreeInput(_) => Err(EvalError::Unsupported(
                "Cannot evaluate free input.".to_string(),
            ))?,
        })
    }

    fn evaluate_reference<'a, T: FieldElement, C: Custom>(
        reference: &'a Reference,
        locals: &[Rc<Value<'a, T, C>>],
        symbols: &impl SymbolLookup<'a, T, C>,
    ) -> Result<Value<'a, T, C>, EvalError> {
        Ok(match reference {
            Reference::LocalVar(i, _name) => (*locals[*i as usize]).clone(),
            Reference::Poly(poly) => symbols.lookup(&poly.name)?,
        })
    }
}

#[cfg(test)]
mod test {
    use number::GoldilocksField;
    use pretty_assertions::assert_eq;

    use crate::analyze_string;

    use super::*;

    fn parse_and_evaluate_symbol(input: &str, symbol: &str) -> String {
        let analyzed = analyze_string::<GoldilocksField>(input);
        let Some(FunctionValueDefinition::Expression(symbol)) = &analyzed.definitions[symbol].1
        else {
            panic!()
        };
        evaluate::<_, NoCustom>(symbol, &Definitions(&analyzed.definitions))
            .unwrap()
            .to_string()
    }

    #[test]
    pub fn arrays_and_strings() {
        let src = r#"namespace Main(16);
            let words = ["the", "quick", "brown", "fox"];
            let translate = |w| match w {
                "the" => "franz",
                "quick" => "jagt",
                "brown" => "mit",
                "fox" => "dem",
                _ => "?",
            };
            let map_array = |arr, f| [f(arr[0]), f(arr[1]), f(arr[2]), f(arr[3])];
            let translated = map_array(words, translate);
        "#;
        let result = parse_and_evaluate_symbol(src, "Main.translated");
        assert_eq!(result, r#"["franz", "jagt", "mit", "dem"]"#);
    }

    #[test]
    pub fn fibonacci() {
        let src = r#"namespace Main(16);
            let fib = |i| match i {
                0 => 0,
                1 => 1,
                _ => fib(i - 1) + fib(i - 2),
            };
            let result = fib(20);
        "#;
        assert_eq!(parse_and_evaluate_symbol(src, "result"), "6765".to_string());
    }

    #[test]
    pub fn capturing() {
        let src = r#"namespace Main(16);
            let f = |n, g| match n { 99 => |i| n, 1 => g(3) };
            let result = f(1, f(99, |x| x + 3000));
        "#;
        // If the lambda function returned by the expression f(99, ...) does not
        // properly capture the value of n in a closure, then f(1, ...) would return 1.
        assert_eq!(parse_and_evaluate_symbol(src, "result"), "99".to_string());
    }
}
