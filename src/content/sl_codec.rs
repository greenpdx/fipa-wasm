// content/sl_codec.rs - FIPA SL (Semantic Language) Codec
//
//! FIPA Semantic Language (SL) codec implementation
//!
//! SL is a Lisp-like content language used in FIPA ACL messages.
//!
//! # Syntax Examples
//!
//! ```text
//! ; Concept
//! (agent-description
//!   :name agent1
//!   :services (set (service-description :name "calc" :type "calculator")))
//!
//! ; Action
//! (action agent1 (register (df-agent-description :name agent1)))
//!
//! ; Predicate
//! (registered agent1 df)
//!
//! ; Proposition (true/false)
//! (= (iota ?x (registered ?x df)) agent1)
//! ```

use super::codec::{Codec, CodecError};
use super::ontology::{Action, Concept, ContentElement, Predicate, Term};
use std::collections::HashMap;

/// FIPA SL Codec
pub struct SlCodec {
    /// Pretty print output
    pretty: bool,
}

impl SlCodec {
    /// Create a new SL codec
    pub fn new() -> Self {
        Self { pretty: false }
    }

    /// Enable pretty printing
    pub fn with_pretty_print(mut self) -> Self {
        self.pretty = true;
        self
    }

    /// Encode a term to SL string
    fn encode_term(&self, term: &Term, output: &mut String) {
        match term {
            Term::String(s) => {
                output.push('"');
                for c in s.chars() {
                    match c {
                        '"' => output.push_str("\\\""),
                        '\\' => output.push_str("\\\\"),
                        '\n' => output.push_str("\\n"),
                        '\t' => output.push_str("\\t"),
                        _ => output.push(c),
                    }
                }
                output.push('"');
            }
            Term::Integer(n) => {
                output.push_str(&n.to_string());
            }
            Term::Float(f) => {
                output.push_str(&f.to_string());
            }
            Term::Boolean(b) => {
                output.push_str(if *b { "true" } else { "false" });
            }
            Term::AgentId(id) => {
                // Agent IDs are unquoted symbols
                output.push_str(id);
            }
            Term::Variable(name) => {
                output.push('?');
                output.push_str(name);
            }
            Term::Concept(concept) => {
                self.encode_concept(concept, output, 0);
            }
            Term::List(items) => {
                output.push_str("(set");
                for item in items {
                    output.push(' ');
                    self.encode_term(item, output);
                }
                output.push(')');
            }
            Term::Null => {
                output.push_str("null");
            }
        }
    }

    /// Encode a concept to SL string
    fn encode_concept(&self, concept: &Concept, output: &mut String, indent: usize) {
        output.push('(');
        output.push_str(&concept.name);

        for (slot_name, slot_value) in &concept.slots {
            if self.pretty {
                output.push('\n');
                for _ in 0..=indent {
                    output.push_str("  ");
                }
            } else {
                output.push(' ');
            }
            output.push(':');
            output.push_str(slot_name);
            output.push(' ');
            self.encode_term(slot_value, output);
        }

        output.push(')');
    }

    /// Encode an action to SL string
    fn encode_action(&self, action: &Action, output: &mut String) {
        output.push_str("(action");

        // Actor
        if let Some(ref actor) = action.actor {
            output.push(' ');
            output.push_str(actor);
        }

        // Action expression
        output.push_str(" (");
        output.push_str(&action.name);

        for (arg_name, arg_value) in &action.arguments {
            if self.pretty {
                output.push('\n');
                output.push_str("    ");
            } else {
                output.push(' ');
            }
            output.push(':');
            output.push_str(arg_name);
            output.push(' ');
            self.encode_term(arg_value, output);
        }

        output.push_str("))");
    }

    /// Encode a predicate to SL string
    fn encode_predicate(&self, predicate: &Predicate, output: &mut String) {
        output.push('(');
        output.push_str(&predicate.name);

        for arg in &predicate.arguments {
            output.push(' ');
            self.encode_term(arg, output);
        }

        output.push(')');
    }
}

impl Default for SlCodec {
    fn default() -> Self {
        Self::new()
    }
}

impl Codec for SlCodec {
    fn language(&self) -> &str {
        "fipa-sl"
    }

    fn name(&self) -> &str {
        "fipa-sl"
    }

    fn encode(&self, content: &ContentElement) -> Result<Vec<u8>, CodecError> {
        let mut output = String::new();

        match content {
            ContentElement::Concept(concept) => {
                self.encode_concept(concept, &mut output, 0);
            }
            ContentElement::Action(action) => {
                self.encode_action(action, &mut output);
            }
            ContentElement::Predicate(predicate) => {
                self.encode_predicate(predicate, &mut output);
            }
            ContentElement::Proposition(inner, value) => {
                if *value {
                    // True proposition - just encode the inner content
                    return self.encode(inner);
                } else {
                    // False proposition - wrap in (not ...)
                    output.push_str("(not ");
                    let inner_bytes = self.encode(inner)?;
                    output.push_str(&String::from_utf8_lossy(&inner_bytes));
                    output.push(')');
                }
            }
            ContentElement::Iota(var, condition) => {
                output.push_str("(iota ?");
                output.push_str(var);
                output.push(' ');
                let cond_bytes = self.encode(condition)?;
                output.push_str(&String::from_utf8_lossy(&cond_bytes));
                output.push(')');
            }
            ContentElement::Any(var, condition) => {
                output.push_str("(any ?");
                output.push_str(var);
                output.push(' ');
                let cond_bytes = self.encode(condition)?;
                output.push_str(&String::from_utf8_lossy(&cond_bytes));
                output.push(')');
            }
            ContentElement::All(var, condition) => {
                output.push_str("(all ?");
                output.push_str(var);
                output.push(' ');
                let cond_bytes = self.encode(condition)?;
                output.push_str(&String::from_utf8_lossy(&cond_bytes));
                output.push(')');
            }
            ContentElement::Sequence(elements) => {
                output.push_str("(sequence");
                for elem in elements {
                    output.push(' ');
                    let elem_bytes = self.encode(elem)?;
                    output.push_str(&String::from_utf8_lossy(&elem_bytes));
                }
                output.push(')');
            }
            ContentElement::Raw(bytes) => {
                // Raw content is encoded as a quoted string
                output.push('"');
                output.push_str(&String::from_utf8_lossy(bytes));
                output.push('"');
            }
        }

        Ok(output.into_bytes())
    }

    fn decode(&self, bytes: &[u8]) -> Result<ContentElement, CodecError> {
        let input = String::from_utf8_lossy(bytes);
        let mut parser = SlParser::new(&input);
        parser.parse()
    }
}

/// SL Parser
struct SlParser<'a> {
    input: &'a str,
    pos: usize,
}

impl<'a> SlParser<'a> {
    fn new(input: &'a str) -> Self {
        Self { input, pos: 0 }
    }

    fn parse(&mut self) -> Result<ContentElement, CodecError> {
        self.skip_whitespace();

        if self.pos >= self.input.len() {
            return Err(CodecError::DecodingFailed("Empty input".to_string()));
        }

        self.parse_expression()
    }

    fn skip_whitespace(&mut self) {
        while self.pos < self.input.len() {
            let c = self.peek_char();
            if c.is_whitespace() {
                self.pos += 1;
            } else if c == ';' {
                // Skip comment to end of line
                while self.pos < self.input.len() && self.peek_char() != '\n' {
                    self.pos += 1;
                }
            } else {
                break;
            }
        }
    }

    fn peek_char(&self) -> char {
        self.input[self.pos..].chars().next().unwrap_or('\0')
    }

    fn next_char(&mut self) -> char {
        let c = self.peek_char();
        if c != '\0' {
            self.pos += c.len_utf8();
        }
        c
    }

    fn parse_expression(&mut self) -> Result<ContentElement, CodecError> {
        self.skip_whitespace();

        let c = self.peek_char();

        if c == '(' {
            self.parse_list_expression()
        } else if c == '"' {
            // String literal - wrap in Raw
            let s = self.parse_string()?;
            Ok(ContentElement::Raw(s.into_bytes()))
        } else {
            // Symbol or number - parse as simple term and wrap
            let term = self.parse_term()?;
            match term {
                Term::Concept(c) => Ok(ContentElement::Concept(*c)),
                _ => Ok(ContentElement::Raw(format!("{:?}", term).into_bytes())),
            }
        }
    }

    fn parse_list_expression(&mut self) -> Result<ContentElement, CodecError> {
        self.expect_char('(')?;
        self.skip_whitespace();

        // Get the head symbol
        let head = self.parse_symbol()?;

        self.skip_whitespace();

        match head.as_str() {
            "action" => self.parse_action_body(),
            "iota" => self.parse_referential("iota"),
            "any" => self.parse_referential("any"),
            "all" => self.parse_referential("all"),
            "not" => self.parse_not(),
            "sequence" | "set" => self.parse_sequence(),
            _ => self.parse_concept_or_predicate(&head),
        }
    }

    fn parse_action_body(&mut self) -> Result<ContentElement, CodecError> {
        self.skip_whitespace();

        // Parse optional actor
        let actor = if self.peek_char() != '(' {
            Some(self.parse_symbol()?)
        } else {
            None
        };

        self.skip_whitespace();

        // Parse action expression
        self.expect_char('(')?;
        self.skip_whitespace();

        let action_name = self.parse_symbol()?;
        let mut arguments = HashMap::new();

        self.skip_whitespace();

        // Parse action arguments (slots)
        while self.peek_char() == ':' {
            let (slot_name, slot_value) = self.parse_slot()?;
            arguments.insert(slot_name, slot_value);
            self.skip_whitespace();
        }

        self.expect_char(')')?;
        self.skip_whitespace();
        self.expect_char(')')?;

        let mut action = Action::new(&action_name);
        action.actor = actor;
        action.arguments = arguments;

        Ok(ContentElement::Action(action))
    }

    fn parse_referential(&mut self, kind: &str) -> Result<ContentElement, CodecError> {
        self.skip_whitespace();

        // Parse variable
        self.expect_char('?')?;
        let var_name = self.parse_symbol()?;

        self.skip_whitespace();

        // Parse condition
        let condition = self.parse_expression()?;

        self.skip_whitespace();
        self.expect_char(')')?;

        match kind {
            "iota" => Ok(ContentElement::Iota(var_name, Box::new(condition))),
            "any" => Ok(ContentElement::Any(var_name, Box::new(condition))),
            "all" => Ok(ContentElement::All(var_name, Box::new(condition))),
            _ => unreachable!(),
        }
    }

    fn parse_not(&mut self) -> Result<ContentElement, CodecError> {
        self.skip_whitespace();
        let inner = self.parse_expression()?;
        self.skip_whitespace();
        self.expect_char(')')?;
        Ok(ContentElement::Proposition(Box::new(inner), false))
    }

    fn parse_sequence(&mut self) -> Result<ContentElement, CodecError> {
        let mut elements = vec![];

        self.skip_whitespace();

        while self.peek_char() != ')' && self.pos < self.input.len() {
            let elem = self.parse_expression()?;
            elements.push(elem);
            self.skip_whitespace();
        }

        self.expect_char(')')?;
        Ok(ContentElement::Sequence(elements))
    }

    fn parse_concept_or_predicate(&mut self, name: &str) -> Result<ContentElement, CodecError> {
        self.skip_whitespace();

        // Check if this looks like a concept (has :slots) or predicate (just args)
        if self.peek_char() == ':' {
            // It's a concept with slots
            let mut slots = HashMap::new();

            while self.peek_char() == ':' {
                let (slot_name, slot_value) = self.parse_slot()?;
                slots.insert(slot_name, slot_value);
                self.skip_whitespace();
            }

            self.expect_char(')')?;

            Ok(ContentElement::Concept(Concept {
                name: name.to_string(),
                slots,
            }))
        } else if self.peek_char() == ')' {
            // Empty concept or predicate
            self.expect_char(')')?;
            Ok(ContentElement::Concept(Concept::new(name)))
        } else {
            // It's a predicate with positional arguments
            let mut arguments = vec![];

            while self.peek_char() != ')' && self.pos < self.input.len() {
                let arg = self.parse_term()?;
                arguments.push(arg);
                self.skip_whitespace();
            }

            self.expect_char(')')?;

            Ok(ContentElement::Predicate(Predicate {
                name: name.to_string(),
                arguments,
            }))
        }
    }

    fn parse_slot(&mut self) -> Result<(String, Term), CodecError> {
        self.expect_char(':')?;
        let name = self.parse_symbol()?;
        self.skip_whitespace();
        let value = self.parse_term()?;
        self.skip_whitespace();
        Ok((name, value))
    }

    fn parse_term(&mut self) -> Result<Term, CodecError> {
        self.skip_whitespace();

        let c = self.peek_char();

        if c == '"' {
            // String
            let s = self.parse_string()?;
            Ok(Term::String(s))
        } else if c == '?' {
            // Variable
            self.next_char();
            let name = self.parse_symbol()?;
            Ok(Term::Variable(name))
        } else if c == '(' {
            // Nested expression (concept or list)
            self.next_char();
            self.skip_whitespace();

            let head = self.parse_symbol()?;
            self.skip_whitespace();

            if head == "set" || head == "sequence" {
                // List
                let mut items = vec![];
                while self.peek_char() != ')' && self.pos < self.input.len() {
                    let item = self.parse_term()?;
                    items.push(item);
                    self.skip_whitespace();
                }
                self.expect_char(')')?;
                Ok(Term::List(items))
            } else {
                // Nested concept
                let mut slots = HashMap::new();

                while self.peek_char() == ':' {
                    let (slot_name, slot_value) = self.parse_slot()?;
                    slots.insert(slot_name, slot_value);
                    self.skip_whitespace();
                }

                self.expect_char(')')?;

                Ok(Term::Concept(Box::new(Concept {
                    name: head,
                    slots,
                })))
            }
        } else if c == '-' || c.is_ascii_digit() {
            // Number
            self.parse_number()
        } else if c == 't' || c == 'f' || c == 'n' {
            // Might be true, false, or null
            let symbol = self.parse_symbol()?;
            match symbol.as_str() {
                "true" => Ok(Term::Boolean(true)),
                "false" => Ok(Term::Boolean(false)),
                "null" => Ok(Term::Null),
                _ => Ok(Term::AgentId(symbol)),
            }
        } else {
            // Symbol (agent ID or other identifier)
            let symbol = self.parse_symbol()?;
            Ok(Term::AgentId(symbol))
        }
    }

    fn parse_string(&mut self) -> Result<String, CodecError> {
        self.expect_char('"')?;

        let mut result = String::new();

        while self.pos < self.input.len() {
            let c = self.next_char();
            if c == '"' {
                return Ok(result);
            } else if c == '\\' {
                // Escape sequence
                let escaped = self.next_char();
                match escaped {
                    'n' => result.push('\n'),
                    't' => result.push('\t'),
                    'r' => result.push('\r'),
                    '"' => result.push('"'),
                    '\\' => result.push('\\'),
                    _ => result.push(escaped),
                }
            } else {
                result.push(c);
            }
        }

        Err(CodecError::SyntaxError {
            position: self.pos,
            message: "Unterminated string".to_string(),
        })
    }

    fn parse_symbol(&mut self) -> Result<String, CodecError> {
        let mut result = String::new();

        while self.pos < self.input.len() {
            let c = self.peek_char();
            if c.is_alphanumeric() || c == '-' || c == '_' || c == '.' {
                result.push(c);
                self.pos += 1;
            } else {
                break;
            }
        }

        if result.is_empty() {
            Err(CodecError::SyntaxError {
                position: self.pos,
                message: format!("Expected symbol, found '{}'", self.peek_char()),
            })
        } else {
            Ok(result)
        }
    }

    fn parse_number(&mut self) -> Result<Term, CodecError> {
        let start = self.pos;
        let mut is_float = false;

        // Handle negative sign
        if self.peek_char() == '-' {
            self.pos += 1;
        }

        // Parse digits
        while self.pos < self.input.len() {
            let c = self.peek_char();
            if c.is_ascii_digit() {
                self.pos += 1;
            } else if c == '.' && !is_float {
                is_float = true;
                self.pos += 1;
            } else if c == 'e' || c == 'E' {
                is_float = true;
                self.pos += 1;
                if self.peek_char() == '+' || self.peek_char() == '-' {
                    self.pos += 1;
                }
            } else {
                break;
            }
        }

        let num_str = &self.input[start..self.pos];

        if is_float {
            num_str
                .parse::<f64>()
                .map(Term::Float)
                .map_err(|e| CodecError::SyntaxError {
                    position: start,
                    message: format!("Invalid float: {}", e),
                })
        } else {
            num_str
                .parse::<i64>()
                .map(Term::Integer)
                .map_err(|e| CodecError::SyntaxError {
                    position: start,
                    message: format!("Invalid integer: {}", e),
                })
        }
    }

    fn expect_char(&mut self, expected: char) -> Result<(), CodecError> {
        let c = self.peek_char();
        if c == expected {
            self.pos += 1;
            Ok(())
        } else {
            Err(CodecError::SyntaxError {
                position: self.pos,
                message: format!("Expected '{}', found '{}'", expected, c),
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_encode_concept() {
        let codec = SlCodec::new();
        let concept = Concept::new("agent-description")
            .with_slot("name", Term::string("my-agent"));

        let encoded = codec.encode(&ContentElement::Concept(concept)).unwrap();
        let result = String::from_utf8(encoded).unwrap();

        assert!(result.contains("agent-description"));
        assert!(result.contains(":name"));
        assert!(result.contains("\"my-agent\""));
    }

    #[test]
    fn test_encode_action() {
        let codec = SlCodec::new();
        let action = Action::new("register")
            .with_actor("agent1")
            .with_arg("service", Term::string("calculator"));

        let encoded = codec.encode(&ContentElement::Action(action)).unwrap();
        let result = String::from_utf8(encoded).unwrap();

        assert!(result.contains("(action agent1"));
        assert!(result.contains("register"));
    }

    #[test]
    fn test_encode_predicate() {
        let codec = SlCodec::new();
        let predicate = Predicate::new("registered")
            .with_arg(Term::agent_id("agent1"))
            .with_arg(Term::agent_id("df"));

        let encoded = codec.encode(&ContentElement::Predicate(predicate)).unwrap();
        let result = String::from_utf8(encoded).unwrap();

        assert_eq!(result, "(registered agent1 df)");
    }

    #[test]
    fn test_decode_concept() {
        let codec = SlCodec::new();
        let input = r#"(agent-description :name "my-agent" :priority 5)"#;

        let result = codec.decode(input.as_bytes()).unwrap();

        if let ContentElement::Concept(concept) = result {
            assert_eq!(concept.name, "agent-description");
            assert_eq!(
                concept.get_slot("name"),
                Some(&Term::String("my-agent".to_string()))
            );
            assert_eq!(concept.get_slot("priority"), Some(&Term::Integer(5)));
        } else {
            panic!("Expected concept");
        }
    }

    #[test]
    fn test_decode_action() {
        let codec = SlCodec::new();
        let input = r#"(action agent1 (register :service "calc"))"#;

        let result = codec.decode(input.as_bytes()).unwrap();

        if let ContentElement::Action(action) = result {
            assert_eq!(action.name, "register");
            assert_eq!(action.actor, Some("agent1".to_string()));
            assert_eq!(
                action.get_arg("service"),
                Some(&Term::String("calc".to_string()))
            );
        } else {
            panic!("Expected action");
        }
    }

    #[test]
    fn test_decode_predicate() {
        let codec = SlCodec::new();
        let input = "(registered agent1 df)";

        let result = codec.decode(input.as_bytes()).unwrap();

        if let ContentElement::Predicate(pred) = result {
            assert_eq!(pred.name, "registered");
            assert_eq!(pred.arguments.len(), 2);
        } else {
            panic!("Expected predicate");
        }
    }

    #[test]
    fn test_roundtrip_concept() {
        let codec = SlCodec::new();
        let original = ContentElement::Concept(
            Concept::new("test-concept")
                .with_slot("string", Term::string("hello"))
                .with_slot("number", Term::integer(42))
                .with_slot("flag", Term::boolean(true)),
        );

        let encoded = codec.encode(&original).unwrap();
        let decoded = codec.decode(&encoded).unwrap();

        if let (ContentElement::Concept(orig), ContentElement::Concept(dec)) = (&original, &decoded)
        {
            assert_eq!(orig.name, dec.name);
            assert_eq!(orig.get_slot("string"), dec.get_slot("string"));
            assert_eq!(orig.get_slot("number"), dec.get_slot("number"));
            assert_eq!(orig.get_slot("flag"), dec.get_slot("flag"));
        } else {
            panic!("Roundtrip failed");
        }
    }

    #[test]
    fn test_nested_concept() {
        let codec = SlCodec::new();
        let input = r#"(outer :inner (nested :value "test"))"#;

        let result = codec.decode(input.as_bytes()).unwrap();

        if let ContentElement::Concept(concept) = result {
            assert_eq!(concept.name, "outer");
            if let Some(Term::Concept(inner)) = concept.get_slot("inner") {
                assert_eq!(inner.name, "nested");
            } else {
                panic!("Expected nested concept");
            }
        } else {
            panic!("Expected concept");
        }
    }

    #[test]
    fn test_list_term() {
        let codec = SlCodec::new();
        let input = r#"(items :list (set "a" "b" "c"))"#;

        let result = codec.decode(input.as_bytes()).unwrap();

        if let ContentElement::Concept(concept) = result {
            if let Some(Term::List(items)) = concept.get_slot("list") {
                assert_eq!(items.len(), 3);
            } else {
                panic!("Expected list");
            }
        } else {
            panic!("Expected concept");
        }
    }

    #[test]
    fn test_iota_expression() {
        let codec = SlCodec::new();
        let input = "(iota ?x (registered ?x df))";

        let result = codec.decode(input.as_bytes()).unwrap();

        if let ContentElement::Iota(var, _condition) = result {
            assert_eq!(var, "x");
        } else {
            panic!("Expected iota expression");
        }
    }
}
