use std::collections::HashMap;

/// An element in the Shuttle Notation parse tree.
#[derive(Debug, Clone, PartialEq)]
pub enum Element {
    Atomic(AtomicElement),
    Section(SectionElement),
    Alternation(AlternationElement),
}

#[derive(Debug, Clone, PartialEq)]
pub struct AtomicElement {
    pub prefix: String,
    pub index: u32,
    pub suffix: String,
    pub repeat: u32,
    pub args: HashMap<String, f64>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SectionElement {
    pub children: Vec<Element>,
    pub repeat: u32,
    pub args: HashMap<String, f64>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct AlternationElement {
    pub arms: Vec<Vec<Element>>,
    pub repeat: u32,
    pub args: HashMap<String, f64>,
}

/// Parse a Shuttle Notation string into a flat sequence of expanded elements.
pub fn parse(source: &str) -> Result<Vec<ResolvedElement>, String> {
    let tokens = tokenize(source);
    let mut cursor = Cursor { tokens: &tokens, pos: 0 };
    let elements = parse_sequence(&mut cursor)?;
    if !cursor.is_done() {
        return Err(format!("Unexpected token at position {}", cursor.pos));
    }
    expand_all(elements)
}

#[derive(Debug, Clone, PartialEq)]
pub struct ResolvedElement {
    pub prefix: String,
    pub index: u32,
    pub suffix: String,
    pub args: HashMap<String, f64>,
}

// -- Tokenizer --

#[derive(Debug, Clone, PartialEq)]
enum Token {
    LParen,
    RParen,
    Slash,
    Star,
    Colon,
    Comma,
    Plus,
    Minus,
    Eq,
    SectionMarker,
    Ident(String),
    Number(String),
}

fn tokenize(source: &str) -> Vec<Token> {
    let mut tokens = Vec::new();
    let mut chars = source.chars().peekable();
    while let Some(&ch) = chars.peek() {
        match ch {
            '(' => { tokens.push(Token::LParen); chars.next(); }
            ')' => { tokens.push(Token::RParen); chars.next(); }
            '/' => { tokens.push(Token::Slash); chars.next(); }
            '*' => { tokens.push(Token::Star); chars.next(); }
            ':' => { tokens.push(Token::Colon); chars.next(); }
            ',' => { tokens.push(Token::Comma); chars.next(); }
            '+' => { tokens.push(Token::Plus); chars.next(); }
            '-' => { tokens.push(Token::Minus); chars.next(); }
            '=' => { tokens.push(Token::Eq); chars.next(); }
            '§' => { tokens.push(Token::SectionMarker); chars.next(); }
            ' ' | '\t' | '\n' | '\r' => { chars.next(); }
            _ if ch.is_ascii_digit() => {
                let mut s = String::new();
                s.push(ch); chars.next();
                while let Some(&c) = chars.peek() {
                    if c.is_ascii_digit() || c == '.' { s.push(c); chars.next(); }
                    else { break; }
                }
                tokens.push(Token::Number(s));
            }
            '.' => {
                // Advance past the dot, then peek ahead to check if it's a decimal
                chars.next(); // consume '.'
                let is_decimal = chars.peek().is_some_and(|c| c.is_ascii_digit());
                if is_decimal {
                    let mut s = String::from(".");
                    while let Some(&c) = chars.peek() {
                        if c.is_ascii_digit() || c == '.' { s.push(c); chars.next(); }
                        else { break; }
                    }
                    tokens.push(Token::Number(s));
                } else {
                    // standalone `.` is legacy alias for `x` (silence/rest)
                    tokens.push(Token::Ident("x".into()));
                }
            }
            _ => {
                let mut s = String::new();
                while let Some(&c) = chars.peek() {
                    if c.is_alphanumeric() || c == '_' || c == '#' { s.push(c); chars.next(); }
                    else { break; }
                }
                if !s.is_empty() {
                    tokens.push(Token::Ident(s));
                } else {
                    chars.next();
                }
            }
        }
    }
    tokens
}

// -- Parser --

struct Cursor<'a> {
    tokens: &'a [Token],
    pos: usize,
}

impl Cursor<'_> {
    fn peek(&self) -> Option<&Token> {
        self.tokens.get(self.pos)
    }
    fn advance(&mut self) {
        self.pos += 1;
    }
    fn expect(&mut self, tok: Token) -> Result<(), String> {
        if self.peek() == Some(&tok) {
            self.advance();
            Ok(())
        } else {
            Err(format!("Expected {:?}, got {:?}", tok, self.peek()))
        }
    }
    fn is_done(&self) -> bool {
        self.pos >= self.tokens.len()
    }
}

/// Parse a space-separated sequence of elements.
fn parse_sequence(cursor: &mut Cursor) -> Result<Vec<Element>, String> {
    let mut elements = Vec::new();
    loop {
        if cursor.peek().is_none()
            || cursor.peek() == Some(&Token::RParen)
            || cursor.peek() == Some(&Token::Slash)
        {
            break;
        }

        // § marks the loop start point for keyboard — structural marker, no sound.
        // Skip it and any following `:N` argument.
        if cursor.peek() == Some(&Token::SectionMarker) {
            cursor.advance();
            if cursor.peek() == Some(&Token::Colon) {
                cursor.advance();
                if let Some(Token::Number(_)) = cursor.peek() {
                    cursor.advance();
                }
            }
            continue;
        }

        elements.push(parse_element(cursor)?);
    }
    Ok(elements)
}

fn parse_element(cursor: &mut Cursor) -> Result<Element, String> {
    match cursor.peek() {
        Some(Token::LParen) => parse_paren_element(cursor),
        Some(Token::Ident(_)) | Some(Token::Number(_)) => parse_atomic(cursor),
        _ => Err(format!("Expected element, got {:?}", cursor.peek())),
    }
}

/// Parse a parenthesized group: either a section or alternation.
fn parse_paren_element(cursor: &mut Cursor) -> Result<Element, String> {
    cursor.expect(Token::LParen)?;

    // Collect arms separated by /
    let mut arms: Vec<Vec<Element>> = Vec::new();
    arms.push(parse_sequence(cursor)?);

    loop {
        match cursor.peek() {
            Some(Token::Slash) => {
                cursor.advance();
                arms.push(parse_sequence(cursor)?);
            }
            Some(Token::RParen) => {
                cursor.advance();
                break;
            }
            _ => return Err(format!("Expected ) or / in group, got {:?}", cursor.peek())),
        }
    }

    let repeat = parse_repeat(cursor);
    let args = parse_args(cursor);

    if arms.len() > 1 {
        Ok(Element::Alternation(AlternationElement { arms, repeat, args }))
    } else if arms.len() == 1 {
        Ok(Element::Section(SectionElement { children: arms.into_iter().next().unwrap(), repeat, args }))
    } else {
        Ok(Element::Section(SectionElement { children: Vec::new(), repeat, args }))
    }
}

fn parse_atomic(cursor: &mut Cursor) -> Result<Element, String> {
    // prefix (ident not starting with digit) or number as index
    let mut prefix = String::new();
    let mut index = 0u32;
    let mut suffix = String::new();

    match cursor.peek() {
        Some(Token::Ident(id)) => {
            // Could be a note name (e.g., c4) or a command prefix
            // Parse it as prefix + number_index + suffix
            let text = id.clone();
            cursor.advance();

            // Split identifier into alpha prefix + numeric index + alpha suffix
            let alpha_prefix: String = text.chars().take_while(|c| c.is_alphabetic() || *c == '_' || *c == '#').collect();
            let rest: String = text.chars().skip(alpha_prefix.len()).collect();
            let digits: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
            let alpha_suffix: String = rest.chars().skip(digits.len()).collect();

            prefix = alpha_prefix;
            if !digits.is_empty() {
                index = digits.parse::<u32>().unwrap_or(0);
            }
            suffix = alpha_suffix;
        }
        Some(Token::Number(n)) => {
            index = n.parse::<u32>().map_err(|e| format!("Invalid number: {}", e))?;
            cursor.advance();
        }
        _ => return Err(format!("Expected atomic element, got {:?}", cursor.peek())),
    }

    let repeat = parse_repeat(cursor);
    let args = parse_args(cursor);

    Ok(Element::Atomic(AtomicElement { prefix, index, suffix, repeat, args }))
}

fn parse_repeat(cursor: &mut Cursor) -> u32 {
    if let Some(Token::Star) = cursor.peek() {
        cursor.advance();
        if let Some(Token::Number(n)) = cursor.peek() {
            let val = n.parse::<u32>().unwrap_or(1);
            cursor.advance();
            val
        } else {
            1
        }
    } else {
        1
    }
}

fn parse_args(cursor: &mut Cursor) -> HashMap<String, f64> {
    let mut args = HashMap::new();
    if cursor.peek() != Some(&Token::Colon) {
        return args;
    }
    cursor.advance(); // consume ':'

    loop {
        // Skip leading commas (empty arg slots like `:,arg1,arg2`)
        while cursor.peek() == Some(&Token::Comma) {
            cursor.advance();
        }

        #[allow(unused_assignments)]
        let mut arg_name = String::new();
        let mut num_str = String::new();

        match cursor.peek() {
            Some(Token::Ident(id)) => {
                arg_name = id.clone();
                cursor.advance();

                let alpha: String = arg_name.chars().take_while(|c| c.is_alphabetic() || *c == '_').collect();
                let trailing: String = arg_name.chars().skip(alpha.len()).collect();
                arg_name = alpha;
                if !trailing.is_empty() {
                    num_str = trailing;
                }
            }
            Some(Token::Number(n)) => {
                num_str = n.clone();
                cursor.advance();
                args.insert("time".to_string(), num_str.parse::<f64>().unwrap_or(0.0));
                if cursor.peek() == Some(&Token::Comma) {
                    cursor.advance();
                    continue;
                }
                break;
            }
            _ => break,
        }

        match cursor.peek() {
            Some(Token::Plus) | Some(Token::Minus) | Some(Token::Star) | Some(Token::Eq) => {
                cursor.advance();
            }
            _ => {}
        }

        match cursor.peek() {
            Some(Token::Number(n)) => {
                num_str.push_str(n);
                cursor.advance();
            }
            _ => {}
        }

        if !arg_name.is_empty() && !num_str.is_empty() {
            let val = num_str.parse::<f64>().unwrap_or(0.0);
            args.insert(arg_name, val);
        }

        if cursor.peek() == Some(&Token::Comma) {
            cursor.advance();
        } else {
            break;
        }
    }

    args
}

// -- Expansion --

fn expand_all(elements: Vec<Element>) -> Result<Vec<ResolvedElement>, String> {
    let mut expander = TreeExpander::new();
    expander.expand_all(elements)
}

struct TreeExpander {
    tick_count: HashMap<usize, u32>,
    next_id: usize,
}

impl TreeExpander {
    fn new() -> Self {
        TreeExpander { tick_count: HashMap::new(), next_id: 0 }
    }

    fn alloc_id(&mut self) -> usize {
        let id = self.next_id;
        self.next_id += 1;
        id
    }

    fn expand_all(&mut self, elements: Vec<Element>) -> Result<Vec<ResolvedElement>, String> {
        let mut result = Vec::new();
        for elem in elements {
            result.extend(self.expand_element(elem, 1)?);
        }
        Ok(result)
    }

    fn expand_element(&mut self, element: Element, repeat: u32) -> Result<Vec<ResolvedElement>, String> {
        match element {
            Element::Atomic(atom) => {
                let mut results = Vec::new();
                for _ in 0..repeat {
                    results.push(ResolvedElement {
                        prefix: atom.prefix.clone(),
                        index: atom.index,
                        suffix: atom.suffix.clone(),
                        args: atom.args.clone(),
                    });
                }
                Ok(results)
            }
            Element::Section(sec) => {
                let mut flat = Vec::new();
                for child in &sec.children {
                    flat.extend(self.expand_element(child.clone(), 1)?);
                }
                let total_repeat = repeat * sec.repeat;
                let mut results = Vec::new();
                for _ in 0..total_repeat {
                    for elem in &flat {
                        results.push(ResolvedElement {
                            prefix: elem.prefix.clone(),
                            index: elem.index,
                            suffix: elem.suffix.clone(),
                            args: merge_args(&sec.args, &elem.args),
                        });
                    }
                }
                Ok(results)
            }
            Element::Alternation(alt) => {
                let id = self.alloc_id();
                let total_repeat = repeat * alt.repeat;
                let mut results = Vec::new();
                for _i in 0..total_repeat as usize {
                    let tick = self.tick_count.entry(id).or_insert(0);
                    let arm_idx = *tick as usize % alt.arms.len();
                    *tick += 1;

                    for child in &alt.arms[arm_idx] {
                        results.extend(self.expand_element(child.clone(), 1)?);
                    }
                }
                Ok(results)
            }
        }
    }
}

fn merge_args(parent: &HashMap<String, f64>, child: &HashMap<String, f64>) -> HashMap<String, f64> {
    let mut merged = parent.clone();
    for (k, v) in child {
        merged.insert(k.clone(), *v);
    }
    merged
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_simple_note() {
        let result = parse("c4").unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].prefix, "c");
        assert_eq!(result[0].index, 4);
    }

    #[test]
    fn test_number_only() {
        let result = parse("14").unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].prefix, "");
        assert_eq!(result[0].index, 14);
    }

    #[test]
    fn test_with_args() {
        let result = parse("c4:amp0.5,sus1.0").unwrap();
        assert_eq!(result[0].prefix, "c");
        assert_eq!(result[0].index, 4);
        assert_eq!(result[0].args.get("amp"), Some(&0.5));
        assert_eq!(result[0].args.get("sus"), Some(&1.0));
    }

    #[test]
    fn test_section() {
        let result = parse("(c4 d4 e4)").unwrap();
        assert_eq!(result.len(), 3);
    }

    #[test]
    fn test_section_with_repeat() {
        let result = parse("(c4 d4)*2").unwrap();
        assert_eq!(result.len(), 4);
    }

    #[test]
    fn test_alternation() {
        let result = parse("(c4 / d4 / e4)*2").unwrap();
        assert_eq!(result.len(), 2);
        assert_eq!(result[0].index, 4);
        assert_eq!(result[1].index, 4); // d4
    }

    #[test]
    fn test_section_with_args() {
        let result = parse("(c4 d4):amp0.5").unwrap();
        assert_eq!(result.len(), 2);
        assert_eq!(result[0].args.get("amp"), Some(&0.5));
        assert_eq!(result[1].args.get("amp"), Some(&0.5));
    }

    #[test]
    fn test_nested_section() {
        let result = parse("((c4 d4) e4)").unwrap();
        assert_eq!(result.len(), 3);
    }
}
