use rusqlite::types::Value;

use crate::error::{CrmError, Result};

#[derive(Debug, Clone, PartialEq)]
enum Token {
    Word(String),
    String(String),
    Number(String),
    Operator(String),
    LeftParen,
    RightParen,
    Comma,
}

pub fn compile(
    input: &str,
    field_sql: impl Fn(&str) -> Option<&'static str>,
) -> Result<(String, Vec<Value>)> {
    let tokens = tokenize(input)?;
    let mut parser = Parser {
        tokens,
        position: 0,
        params: Vec::new(),
        field_sql: &field_sql,
    };
    let sql = parser.parse_or()?;
    if parser.position != parser.tokens.len() {
        return Err(CrmError::InvalidQuery("unexpected trailing input".into()));
    }
    Ok((sql, parser.params))
}

struct Parser<'a, F> {
    tokens: Vec<Token>,
    position: usize,
    params: Vec<Value>,
    field_sql: &'a F,
}

impl<F> Parser<'_, F>
where
    F: Fn(&str) -> Option<&'static str>,
{
    fn parse_or(&mut self) -> Result<String> {
        let mut expressions = vec![self.parse_and()?];
        while self.take_word("or") {
            expressions.push(self.parse_and()?);
        }
        Ok(join(expressions, " OR "))
    }

    fn parse_and(&mut self) -> Result<String> {
        let mut expressions = vec![self.parse_unary()?];
        while self.take_word("and") {
            expressions.push(self.parse_unary()?);
        }
        Ok(join(expressions, " AND "))
    }

    fn parse_unary(&mut self) -> Result<String> {
        if self.take_word("not") {
            return Ok(format!("(NOT {})", self.parse_unary()?));
        }
        if self.take(&Token::LeftParen) {
            let expression = self.parse_or()?;
            self.expect(Token::RightParen)?;
            return Ok(expression);
        }
        self.parse_predicate()
    }

    fn parse_predicate(&mut self) -> Result<String> {
        let field = match self.next() {
            Some(Token::Word(field)) => field,
            _ => return Err(CrmError::InvalidQuery("expected a field name".into())),
        };
        let sql_field = (self.field_sql)(&field)
            .ok_or_else(|| CrmError::InvalidQuery(format!("unknown field `{field}`")))?;
        if self.take_word("is") {
            let negated = self.take_word("not");
            if !self.take_word("null") {
                return Err(CrmError::InvalidQuery("expected null after is".into()));
            }
            return Ok(format!(
                "({sql_field} IS {}NULL)",
                if negated { "NOT " } else { "" }
            ));
        }
        if self.take_word("contains") {
            let value = self.value()?;
            self.params
                .push(Value::Text(format!("%{}%", value_text(value))));
            return Ok(format!("({sql_field} LIKE ? ESCAPE '\\')"));
        }
        if self.take_word("in") {
            self.expect(Token::LeftParen)?;
            let mut placeholders = Vec::new();
            loop {
                let value = self.value()?;
                self.params.push(value);
                placeholders.push("?");
                if !self.take(&Token::Comma) {
                    break;
                }
            }
            self.expect(Token::RightParen)?;
            return Ok(format!("({sql_field} IN ({}))", placeholders.join(", ")));
        }
        let operator = match self.next() {
            Some(Token::Operator(operator))
                if matches!(operator.as_str(), "=" | "!=" | "<" | "<=" | ">" | ">=") =>
            {
                operator
            }
            _ => {
                return Err(CrmError::InvalidQuery(
                    "expected a comparison operator".into(),
                ));
            }
        };
        let value = self.value()?;
        self.params.push(value);
        Ok(format!("({sql_field} {operator} ?)"))
    }

    fn value(&mut self) -> Result<Value> {
        match self.next() {
            Some(Token::String(value)) => Ok(Value::Text(value)),
            Some(Token::Number(value)) => number_or_duration(&value),
            Some(Token::Word(value)) if safe_bare_value(&value) => Ok(Value::Text(value)),
            _ => Err(CrmError::InvalidQuery("expected a value".into())),
        }
    }

    fn next(&mut self) -> Option<Token> {
        let token = self.tokens.get(self.position).cloned();
        self.position += usize::from(token.is_some());
        token
    }

    fn take(&mut self, expected: &Token) -> bool {
        if self.tokens.get(self.position) == Some(expected) {
            self.position += 1;
            true
        } else {
            false
        }
    }

    fn take_word(&mut self, expected: &str) -> bool {
        match self.tokens.get(self.position) {
            Some(Token::Word(word)) if word.eq_ignore_ascii_case(expected) => {
                self.position += 1;
                true
            }
            _ => false,
        }
    }

    fn expect(&mut self, expected: Token) -> Result<()> {
        if self.take(&expected) {
            Ok(())
        } else {
            Err(CrmError::InvalidQuery(format!("expected {expected:?}")))
        }
    }
}

fn tokenize(input: &str) -> Result<Vec<Token>> {
    let mut tokens = Vec::new();
    let mut chars = input.char_indices().peekable();
    while let Some((_, character)) = chars.next() {
        if character.is_whitespace() {
            continue;
        }
        match character {
            '(' => tokens.push(Token::LeftParen),
            ')' => tokens.push(Token::RightParen),
            ',' => tokens.push(Token::Comma),
            '=' | '<' | '>' | '!' => {
                let mut operator = character.to_string();
                if matches!(chars.peek(), Some((_, '='))) {
                    operator.push('=');
                    chars.next();
                }
                tokens.push(Token::Operator(operator));
            }
            '\'' | '"' => {
                let quote = character;
                let mut value = String::new();
                let mut closed = false;
                for (_, next) in chars.by_ref() {
                    if next == quote {
                        closed = true;
                        break;
                    }
                    value.push(next);
                }
                if !closed {
                    return Err(CrmError::InvalidQuery("unterminated string".into()));
                }
                tokens.push(Token::String(value));
            }
            _ => {
                let mut value = character.to_string();
                while let Some((_, next)) = chars.peek() {
                    if next.is_whitespace() || "(),=<>!".contains(*next) {
                        break;
                    }
                    value.push(*next);
                    chars.next();
                }
                if value
                    .chars()
                    .next()
                    .is_some_and(|first| first.is_ascii_digit())
                {
                    tokens.push(Token::Number(value));
                } else {
                    tokens.push(Token::Word(value));
                }
            }
        }
    }
    Ok(tokens)
}

fn number_or_duration(value: &str) -> Result<Value> {
    if let Some(days) = value.strip_suffix('d') {
        return days
            .parse::<i64>()
            .map(Value::Integer)
            .map_err(|_| CrmError::InvalidQuery(format!("invalid duration `{value}`")));
    }
    if let Some(months) = value.strip_suffix("mo") {
        return months
            .parse::<i64>()
            .map(|months| Value::Integer(months * 30))
            .map_err(|_| CrmError::InvalidQuery(format!("invalid duration `{value}`")));
    }
    if let Ok(integer) = value.parse::<i64>() {
        return Ok(Value::Integer(integer));
    }
    value
        .parse::<f64>()
        .map(Value::Real)
        .map_err(|_| CrmError::InvalidQuery(format!("invalid number `{value}`")))
}

fn value_text(value: Value) -> String {
    match value {
        Value::Text(value) => value,
        Value::Integer(value) => value.to_string(),
        Value::Real(value) => value.to_string(),
        _ => String::new(),
    }
}

fn safe_bare_value(value: &str) -> bool {
    value
        .chars()
        .all(|character| character.is_alphanumeric() || "_-.:@+".contains(character))
}
fn join(expressions: Vec<String>, operator: &str) -> String {
    if expressions.len() == 1 {
        expressions.into_iter().next().unwrap()
    } else {
        format!("({})", expressions.join(operator))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn field(name: &str) -> Option<&'static str> {
        match name {
            "name" => Some("p.display_name"),
            "days" => Some("days"),
            _ => None,
        }
    }

    #[test]
    fn compiles_boolean_expression_with_parameters() {
        let (sql, params) = compile(
            "name contains 'Alice' and (days > 30d or days is null)",
            field,
        )
        .unwrap();
        assert_eq!(
            sql,
            "((p.display_name LIKE ? ESCAPE '\\') AND ((days > ?) OR (days IS NULL)))"
        );
        assert_eq!(params, [Value::Text("%Alice%".into()), Value::Integer(30)]);
    }

    #[test]
    fn rejects_unknown_fields() {
        assert!(compile("secret = 1", field).is_err());
    }
}
