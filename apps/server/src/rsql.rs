//! RSQL (FIQL) parser → parameterized SQL, hawkBit `q=` compatible.
//!
//! Grammar (hand-rolled recursive descent — no dependency justifies itself):
//!   or    := and ( "," and )*
//!   and   := cmp ( ";" cmp )*
//!   cmp   := "(" or ")" | selector op args
//!   op    := == != =in= =out= =lt= =le= =gt= =ge= < <= > >=
//!   args  := value | "(" value ("," value)* ")"
//!
//! `*` wildcards in ==/!= become case-insensitive ILIKE patterns (hawkBit's
//! behaviour for wildcard string matching).

use std::collections::BTreeMap;

use crate::error::{AppError, AppResult};

#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Text(String),
    Int(i64),
    Bool(bool),
}

/// How a selector maps to SQL.
#[derive(Debug, Clone)]
pub enum Field {
    /// Plain column on the queried table (alias included), e.g. `t.name`.
    Column { sql: &'static str, kind: Kind },
    /// Enum column stored as INTEGER with a name → code map.
    EnumColumn {
        sql: &'static str,
        map: &'static [(&'static str, i32)],
    },
    /// `attribute.<key>` / `metadata.<key>` style sub-select:
    /// EXISTS (SELECT 1 FROM {table} x WHERE x.{fk} = {owner} AND x.{key_col} = <key> AND x.{val_col} OP <value>)
    KeyValueSub {
        table: &'static str,
        fk: &'static str,
        owner: &'static str,
        key_col: &'static str,
        val_col: &'static str,
    },
    /// Tag name sub-select: EXISTS(... join tag table ...).
    Exists { template: &'static str },
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Kind {
    Text,
    Int,
    Bool,
}

/// Per-resource selector map. Keys are lowercase; `attribute.*`/`metadata.*`
/// prefixes are matched by the part before the first dot.
pub struct FieldMap {
    pub fields: BTreeMap<&'static str, Field>,
}

impl FieldMap {
    pub fn new(entries: Vec<(&'static str, Field)>) -> Self {
        Self {
            fields: entries.into_iter().collect(),
        }
    }

    /// Borrow another map's selectors, keeping the ones already declared here.
    /// Lets a resource share a common map and add the columns only its own
    /// table has.
    pub fn extend(&mut self, other: &FieldMap) {
        for (name, field) in &other.fields {
            self.fields.entry(name).or_insert_with(|| field.clone());
        }
    }

    fn resolve(&self, selector: &str) -> AppResult<(Field, Option<String>)> {
        let lower = selector.to_lowercase();
        if let Some(field) = self.fields.get(lower.as_str()) {
            return Ok((field.clone(), None));
        }
        // prefixed selector: attribute.<key>, metadata.<key>, targettype.key ...
        if let Some((prefix, rest)) = lower.split_once('.') {
            if let Some(field) = self.fields.get(format!("{prefix}.*").as_str()) {
                return Ok((field.clone(), Some(rest.to_string())));
            }
            if let Some(field) = self.fields.get(lower.as_str()) {
                return Ok((field.clone(), None));
            }
        }
        Err(AppError::InvalidRsql(format!("unknown field '{selector}'")))
    }
}

// ---------------------------------------------------------------------------
// Lexer / parser
// ---------------------------------------------------------------------------

#[derive(Debug, PartialEq, Clone)]
enum Op {
    Eq,
    Ne,
    In,
    Out,
    Lt,
    Le,
    Gt,
    Ge,
}

#[derive(Debug)]
enum Node {
    Or(Vec<Node>),
    And(Vec<Node>),
    Cmp {
        selector: String,
        op: Op,
        args: Vec<String>,
    },
}

struct Parser<'a> {
    input: &'a [u8],
    pos: usize,
}

impl<'a> Parser<'a> {
    fn new(input: &'a str) -> Self {
        Self {
            input: input.as_bytes(),
            pos: 0,
        }
    }

    fn err(&self, message: &str) -> AppError {
        AppError::InvalidRsql(format!("{message} at position {}", self.pos))
    }

    fn peek(&self) -> Option<u8> {
        self.input.get(self.pos).copied()
    }

    fn eat(&mut self, byte: u8) -> bool {
        if self.peek() == Some(byte) {
            self.pos += 1;
            true
        } else {
            false
        }
    }

    fn parse_or(&mut self) -> AppResult<Node> {
        let mut nodes = vec![self.parse_and()?];
        while self.eat(b',') {
            nodes.push(self.parse_and()?);
        }
        Ok(if nodes.len() == 1 {
            nodes.pop().unwrap()
        } else {
            Node::Or(nodes)
        })
    }

    fn parse_and(&mut self) -> AppResult<Node> {
        let mut nodes = vec![self.parse_cmp()?];
        while self.eat(b';') {
            nodes.push(self.parse_cmp()?);
        }
        Ok(if nodes.len() == 1 {
            nodes.pop().unwrap()
        } else {
            Node::And(nodes)
        })
    }

    fn parse_cmp(&mut self) -> AppResult<Node> {
        if self.eat(b'(') {
            let node = self.parse_or()?;
            if !self.eat(b')') {
                return Err(self.err("expected ')'"));
            }
            return Ok(node);
        }
        let selector = self.parse_selector()?;
        let op = self.parse_op()?;
        let args = self.parse_args()?;
        Ok(Node::Cmp { selector, op, args })
    }

    fn parse_selector(&mut self) -> AppResult<String> {
        let start = self.pos;
        while let Some(c) = self.peek() {
            if c.is_ascii_alphanumeric() || c == b'.' || c == b'_' || c == b'-' {
                self.pos += 1;
            } else {
                break;
            }
        }
        if self.pos == start {
            return Err(self.err("expected field selector"));
        }
        Ok(String::from_utf8_lossy(&self.input[start..self.pos]).into_owned())
    }

    fn parse_op(&mut self) -> AppResult<Op> {
        let rest = &self.input[self.pos..];
        let table: &[(&[u8], Op)] = &[
            (b"==", Op::Eq),
            (b"!=", Op::Ne),
            (b"=in=", Op::In),
            (b"=out=", Op::Out),
            (b"=lt=", Op::Lt),
            (b"=le=", Op::Le),
            (b"=gt=", Op::Gt),
            (b"=ge=", Op::Ge),
            (b"<=", Op::Le),
            (b">=", Op::Ge),
            (b"<", Op::Lt),
            (b">", Op::Gt),
        ];
        for (token, op) in table {
            if rest.starts_with(token) {
                self.pos += token.len();
                return Ok(op.clone());
            }
        }
        Err(self.err("expected comparison operator"))
    }

    fn parse_args(&mut self) -> AppResult<Vec<String>> {
        if self.eat(b'(') {
            let mut args = vec![self.parse_value()?];
            while self.eat(b',') {
                args.push(self.parse_value()?);
            }
            if !self.eat(b')') {
                return Err(self.err("expected ')' after value list"));
            }
            Ok(args)
        } else {
            Ok(vec![self.parse_value()?])
        }
    }

    fn parse_value(&mut self) -> AppResult<String> {
        match self.peek() {
            Some(quote @ (b'"' | b'\'')) => {
                self.pos += 1;
                let mut out = Vec::new();
                loop {
                    match self.peek() {
                        Some(c) if c == quote => {
                            self.pos += 1;
                            break;
                        }
                        Some(b'\\') => {
                            self.pos += 1;
                            if let Some(escaped) = self.peek() {
                                out.push(escaped);
                                self.pos += 1;
                            }
                        }
                        Some(c) => {
                            out.push(c);
                            self.pos += 1;
                        }
                        None => return Err(self.err("unterminated quoted value")),
                    }
                }
                Ok(String::from_utf8_lossy(&out).into_owned())
            }
            _ => {
                let start = self.pos;
                while let Some(c) = self.peek() {
                    if matches!(c, b',' | b';' | b'(' | b')') {
                        break;
                    }
                    self.pos += 1;
                }
                if self.pos == start {
                    return Err(self.err("expected value"));
                }
                Ok(String::from_utf8_lossy(&self.input[start..self.pos])
                    .trim()
                    .to_string())
            }
        }
    }
}

// ---------------------------------------------------------------------------
// SQL generation
// ---------------------------------------------------------------------------

pub struct WhereClause {
    /// SQL fragment with `$N` placeholders (to AND onto the base query).
    pub sql: String,
    pub binds: Vec<Value>,
}

struct SqlBuilder<'m> {
    map: &'m FieldMap,
    binds: Vec<Value>,
    /// First placeholder index to use ($1 is usually taken by tenant).
    offset: usize,
}

impl<'m> SqlBuilder<'m> {
    fn push_bind(&mut self, value: Value) -> String {
        self.binds.push(value);
        format!("${}", self.offset + self.binds.len())
    }

    fn build(&mut self, node: &Node) -> AppResult<String> {
        match node {
            Node::Or(children) => {
                let parts: Vec<String> = children
                    .iter()
                    .map(|c| self.build(c))
                    .collect::<AppResult<_>>()?;
                Ok(format!("({})", parts.join(" OR ")))
            }
            Node::And(children) => {
                let parts: Vec<String> = children
                    .iter()
                    .map(|c| self.build(c))
                    .collect::<AppResult<_>>()?;
                Ok(format!("({})", parts.join(" AND ")))
            }
            Node::Cmp { selector, op, args } => self.build_cmp(selector, op, args),
        }
    }

    fn coerce(&self, kind: Kind, raw: &str, selector: &str) -> AppResult<Value> {
        match kind {
            Kind::Text => Ok(Value::Text(raw.to_string())),
            Kind::Int => raw
                .parse::<i64>()
                .map(Value::Int)
                .map_err(|_| AppError::InvalidRsql(format!("'{selector}' expects a number"))),
            Kind::Bool => match raw.to_ascii_lowercase().as_str() {
                "true" => Ok(Value::Bool(true)),
                "false" => Ok(Value::Bool(false)),
                _ => Err(AppError::InvalidRsql(format!(
                    "'{selector}' expects true/false"
                ))),
            },
        }
    }

    fn build_cmp(&mut self, selector: &str, op: &Op, args: &[String]) -> AppResult<String> {
        let (field, key) = self.map.resolve(selector)?;
        let value_expr = |builder: &mut Self, sql_col: &str, kind: Kind| -> AppResult<String> {
            let raw = &args[0];
            match op {
                // `field==null` / `field!=null` — the only way to ask for an
                // unset column (hawkBit has no null literal either, but its
                // absence makes "unassigned"-style filters impossible).
                Op::Eq | Op::Ne if raw.eq_ignore_ascii_case("null") => Ok(format!(
                    "{sql_col} IS {}NULL",
                    if *op == Op::Eq { "" } else { "NOT " }
                )),
                Op::Eq | Op::Ne => {
                    if kind == Kind::Text && raw.contains('*') {
                        let pattern = raw
                            .replace('%', "\\%")
                            .replace('_', "\\_")
                            .replace('*', "%");
                        let ph = builder.push_bind(Value::Text(pattern));
                        let cmp = if *op == Op::Eq { "ILIKE" } else { "NOT ILIKE" };
                        Ok(format!("{sql_col} {cmp} {ph}"))
                    } else {
                        let ph = builder.push_bind(builder.coerce(kind, raw, selector)?);
                        let cmp = if *op == Op::Eq { "=" } else { "<>" };
                        Ok(format!("{sql_col} {cmp} {ph}"))
                    }
                }
                Op::In | Op::Out => {
                    let mut placeholders = Vec::with_capacity(args.len());
                    for arg in args {
                        let ph = builder.push_bind(builder.coerce(kind, arg, selector)?);
                        placeholders.push(ph);
                    }
                    let cmp = if *op == Op::In { "IN" } else { "NOT IN" };
                    Ok(format!("{sql_col} {cmp} ({})", placeholders.join(", ")))
                }
                Op::Lt | Op::Le | Op::Gt | Op::Ge => {
                    let ph = builder.push_bind(builder.coerce(kind, raw, selector)?);
                    let cmp = match op {
                        Op::Lt => "<",
                        Op::Le => "<=",
                        Op::Gt => ">",
                        _ => ">=",
                    };
                    Ok(format!("{sql_col} {cmp} {ph}"))
                }
            }
        };

        match field {
            Field::Column { sql, kind } => value_expr(self, sql, kind),
            Field::EnumColumn { sql, map } => {
                let to_code = |raw: &str| -> AppResult<Value> {
                    let lower = raw.to_lowercase();
                    map.iter()
                        .find(|(name, _)| *name == lower)
                        .map(|(_, code)| Value::Int(*code as i64))
                        .ok_or_else(|| {
                            AppError::InvalidRsql(format!("unknown value '{raw}' for '{selector}'"))
                        })
                };
                match op {
                    Op::Eq | Op::Ne => {
                        let ph = self.push_bind(to_code(&args[0])?);
                        Ok(format!(
                            "{sql} {} {ph}",
                            if *op == Op::Eq { "=" } else { "<>" }
                        ))
                    }
                    Op::In | Op::Out => {
                        let mut placeholders = Vec::new();
                        for arg in args {
                            let code = to_code(arg)?;
                            placeholders.push(self.push_bind(code));
                        }
                        let cmp = if *op == Op::In { "IN" } else { "NOT IN" };
                        Ok(format!("{sql} {cmp} ({})", placeholders.join(", ")))
                    }
                    _ => Err(AppError::InvalidRsql(format!(
                        "operator not supported for enum field '{selector}'"
                    ))),
                }
            }
            Field::KeyValueSub {
                table,
                fk,
                owner,
                key_col,
                val_col,
            } => {
                // NARROW: the value column is VARCHAR, and the bind below is
                // always Value::Text — including for =lt=/=le=/=gt=/=ge=. A
                // numeric attribute is therefore compared lexicographically:
                // `attribute.bucket=le=50000` keeps '9' and drops '10000'.
                // There is no error, only a wrong answer. See SCHEMA_NOTES.md
                // #21; a numeric dimension belongs in a hawier_* sidecar until
                // attribute values are typed.
                let key = key.ok_or_else(|| {
                    AppError::InvalidRsql(format!("'{selector}' needs a key, e.g. {selector}.key"))
                })?;
                let key_ph = self.push_bind(Value::Text(key));
                let raw = &args[0];
                let (cmp, value) = if raw.contains('*') {
                    (
                        if *op == Op::Ne { "NOT ILIKE" } else { "ILIKE" },
                        Value::Text(raw.replace('*', "%")),
                    )
                } else {
                    (
                        match op {
                            Op::Eq => "=",
                            Op::Ne => "<>",
                            Op::Lt => "<",
                            Op::Le => "<=",
                            Op::Gt => ">",
                            Op::Ge => ">=",
                            _ => {
                                return Err(AppError::InvalidRsql(format!(
                                    "operator not supported for '{selector}'"
                                )))
                            }
                        },
                        Value::Text(raw.clone()),
                    )
                };
                let val_ph = self.push_bind(value);
                Ok(format!(
                    "EXISTS (SELECT 1 FROM {table} kv WHERE kv.{fk} = {owner} \
                     AND kv.{key_col} = {key_ph} AND kv.{val_col} {cmp} {val_ph})"
                ))
            }
            Field::Exists { template } => {
                // template contains "$V" where the value placeholder goes;
                // wildcard support included.
                let raw = &args[0];
                let (cmp, value) = if raw.contains('*') {
                    ("ILIKE", Value::Text(raw.replace('*', "%")))
                } else {
                    ("=", Value::Text(raw.clone()))
                };
                let ph = self.push_bind(value);
                let exists = template.replace("$CMP", cmp).replace("$V", &ph);
                match op {
                    Op::Eq | Op::In => Ok(exists),
                    Op::Ne | Op::Out => Ok(format!("NOT {exists}")),
                    _ => Err(AppError::InvalidRsql(format!(
                        "operator not supported for '{selector}'"
                    ))),
                }
            }
        }
    }
}

/// Parse `q` and produce a WHERE fragment. `bind_offset` = number of
/// placeholders already used by the caller ($1..$offset).
pub fn to_sql(query: &str, map: &FieldMap, bind_offset: usize) -> AppResult<WhereClause> {
    let mut parser = Parser::new(query);
    let node = parser.parse_or()?;
    if parser.pos != parser.input.len() {
        return Err(parser.err("unexpected trailing input"));
    }
    let mut builder = SqlBuilder {
        map,
        binds: Vec::new(),
        offset: bind_offset,
    };
    let sql = builder.build(&node)?;
    Ok(WhereClause {
        sql,
        binds: builder.binds,
    })
}

/// Bind the produced values onto a sqlx query.
pub fn bind_values<'q>(
    mut query: sqlx::query::Query<'q, sqlx::Postgres, sqlx::postgres::PgArguments>,
    binds: &'q [Value],
) -> sqlx::query::Query<'q, sqlx::Postgres, sqlx::postgres::PgArguments> {
    for value in binds {
        query = match value {
            Value::Text(v) => query.bind(v),
            Value::Int(v) => query.bind(v),
            Value::Bool(v) => query.bind(v),
        };
    }
    query
}

#[cfg(test)]
mod tests {
    use super::*;

    fn target_map() -> FieldMap {
        FieldMap::new(vec![
            (
                "name",
                Field::Column {
                    sql: "t.name",
                    kind: Kind::Text,
                },
            ),
            (
                "controllerid",
                Field::Column {
                    sql: "t.controller_id",
                    kind: Kind::Text,
                },
            ),
            (
                "updatestatus",
                Field::EnumColumn {
                    sql: "t.update_status",
                    map: &[
                        ("unknown", 0),
                        ("in_sync", 1),
                        ("pending", 2),
                        ("error", 3),
                        ("registered", 4),
                    ],
                },
            ),
            (
                "attribute.*",
                Field::KeyValueSub {
                    table: "sp_target_attributes",
                    fk: "target",
                    owner: "t.id",
                    key_col: "attribute_key",
                    val_col: "attribute_value",
                },
            ),
        ])
    }

    #[test]
    fn simple_eq() {
        let clause = to_sql("name==foo", &target_map(), 1).unwrap();
        assert_eq!(clause.sql, "t.name = $2");
        assert_eq!(clause.binds, vec![Value::Text("foo".into())]);
    }

    #[test]
    fn wildcard_becomes_ilike() {
        let clause = to_sql("name==t*", &target_map(), 1).unwrap();
        assert_eq!(clause.sql, "t.name ILIKE $2");
        assert_eq!(clause.binds, vec![Value::Text("t%".into())]);
    }

    #[test]
    fn and_or_grouping() {
        let clause = to_sql("(name==a,name==b);controllerid!=c", &target_map(), 0).unwrap();
        assert_eq!(
            clause.sql,
            "((t.name = $1 OR t.name = $2) AND t.controller_id <> $3)"
        );
        assert_eq!(clause.binds.len(), 3);
    }

    #[test]
    fn enum_name_translated_to_code() {
        let clause = to_sql("updatestatus==pending", &target_map(), 0).unwrap();
        assert_eq!(clause.sql, "t.update_status = $1");
        assert_eq!(clause.binds, vec![Value::Int(2)]);
    }

    #[test]
    fn in_list() {
        let clause = to_sql("updatestatus=in=(pending,error)", &target_map(), 0).unwrap();
        assert_eq!(clause.sql, "t.update_status IN ($1, $2)");
        assert_eq!(clause.binds, vec![Value::Int(2), Value::Int(3)]);
    }

    #[test]
    fn null_literal_maps_to_is_null() {
        let clause = to_sql("name==null", &target_map(), 0).unwrap();
        assert_eq!(clause.sql, "t.name IS NULL");
        assert!(clause.binds.is_empty(), "no bind for a null test");

        let clause = to_sql("name!=NULL", &target_map(), 0).unwrap();
        assert_eq!(clause.sql, "t.name IS NOT NULL");

        // Int columns accept it too — that is the point (assignedDistributionSet).
        let clause = to_sql("controllerid==null;name!=null", &target_map(), 0).unwrap();
        assert_eq!(
            clause.sql,
            "(t.controller_id IS NULL AND t.name IS NOT NULL)"
        );
    }

    #[test]
    fn attribute_subquery() {
        let clause = to_sql("attribute.hw==rev1", &target_map(), 0).unwrap();
        assert!(clause
            .sql
            .contains("EXISTS (SELECT 1 FROM sp_target_attributes"));
        assert_eq!(
            clause.binds,
            vec![Value::Text("hw".into()), Value::Text("rev1".into())]
        );
    }

    #[test]
    fn quoted_values_and_errors() {
        let clause = to_sql("name=='hello world'", &target_map(), 0).unwrap();
        assert_eq!(clause.binds, vec![Value::Text("hello world".into())]);
        assert!(to_sql("bogus==1", &target_map(), 0).is_err());
        assert!(to_sql("name==", &target_map(), 0).is_err());
        assert!(to_sql("name==a,,", &target_map(), 0).is_err());
    }
}
