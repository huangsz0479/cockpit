use crate::{RiskLevel, SqlAssessment};

pub fn assess_sql(sql: &str) -> SqlAssessment {
    let statements = split_statements(sql);
    if statements.len() <= 1 {
        return assess_statement(statements.first().map_or(sql, String::as_str));
    }
    let assessments = statements
        .iter()
        .map(|statement| assess_statement(statement))
        .collect::<Vec<_>>();
    let risk = assessments
        .iter()
        .map(|assessment| &assessment.risk)
        .max_by_key(|risk| risk_weight(risk))
        .cloned()
        .unwrap_or(RiskLevel::Review);
    let requires_confirmation = assessments
        .iter()
        .any(|assessment| assessment.requires_confirmation);
    let reason = requires_confirmation.then(|| {
        let detail = assessments
            .iter()
            .find_map(|assessment| assessment.reason.as_deref())
            .unwrap_or("脚本包含会改变状态的语句");
        format!("脚本包含 {} 条语句；{detail}", statements.len())
    });
    SqlAssessment {
        statement_kind: "MULTI_STATEMENT".into(),
        risk,
        requires_confirmation,
        reason,
    }
}

fn assess_statement(sql: &str) -> SqlAssessment {
    let normalized = strip_leading_trivia(sql);
    let tokens = sql_tokens(normalized);
    let keyword = tokens.first().cloned().unwrap_or_default();
    let upper = normalized.to_ascii_uppercase();

    let (risk, requires_confirmation, reason) = match keyword.as_str() {
        "SELECT" if select_has_side_effects(&tokens) => (
            RiskLevel::Review,
            true,
            Some("SELECT 包含文件写入、锁或会话状态修改".to_string()),
        ),
        "EXPLAIN" if tokens.get(1).is_some_and(|token| token == "ANALYZE") => (
            RiskLevel::Review,
            true,
            Some("EXPLAIN ANALYZE 会实际执行目标语句".to_string()),
        ),
        "SELECT" | "SHOW" | "DESCRIBE" | "DESC" | "EXPLAIN" => (RiskLevel::Safe, false, None),
        "WITH" => (
            RiskLevel::Review,
            true,
            Some("CTE 可能包含数据修改，当前版本按保守策略处理".to_string()),
        ),
        "INSERT" | "REPLACE" => (RiskLevel::Review, true, Some("将写入数据".to_string())),
        "UPDATE" | "DELETE" if !contains_keyword(&upper, "WHERE") => (
            RiskLevel::Destructive,
            true,
            Some("UPDATE/DELETE 未包含 WHERE 条件".to_string()),
        ),
        "UPDATE" | "DELETE" => (RiskLevel::Review, true, Some("将修改现有数据".to_string())),
        "DROP" | "TRUNCATE" => (
            RiskLevel::Destructive,
            true,
            Some("将删除数据库对象或数据".to_string()),
        ),
        "ALTER" | "CREATE" | "RENAME" => (
            RiskLevel::Review,
            true,
            Some("将修改数据库结构".to_string()),
        ),
        "GRANT" | "REVOKE" | "SET" => (
            RiskLevel::Review,
            true,
            Some("将修改权限或会话状态".to_string()),
        ),
        _ => (
            RiskLevel::Review,
            true,
            Some("无法可靠判断该语句的影响".to_string()),
        ),
    };

    SqlAssessment {
        statement_kind: keyword,
        risk,
        requires_confirmation,
        reason,
    }
}

fn select_has_side_effects(tokens: &[String]) -> bool {
    tokens.iter().any(|token| {
        matches!(
            token.as_str(),
            "INTO" | "GET_LOCK" | "RELEASE_LOCK" | "RELEASE_ALL_LOCKS" | "SLEEP"
        )
    }) || contains_token_sequence(tokens, &["FOR", "UPDATE"])
        || contains_token_sequence(tokens, &["FOR", "SHARE"])
        || contains_token_sequence(tokens, &["LOCK", "IN", "SHARE", "MODE"])
}

fn contains_token_sequence(tokens: &[String], expected: &[&str]) -> bool {
    tokens.windows(expected.len()).any(|window| {
        window
            .iter()
            .map(String::as_str)
            .eq(expected.iter().copied())
    })
}

fn sql_tokens(sql: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut token = String::new();
    let mut chars = sql.chars().peekable();
    let mut state = LexerState::Normal;
    while let Some(character) = chars.next() {
        match state {
            LexerState::Normal => match character {
                '\'' => {
                    push_token(&mut tokens, &mut token);
                    state = LexerState::SingleQuote;
                }
                '"' => {
                    push_token(&mut tokens, &mut token);
                    state = LexerState::DoubleQuote;
                }
                '`' => {
                    push_token(&mut tokens, &mut token);
                    state = LexerState::Backtick;
                }
                '#' => {
                    push_token(&mut tokens, &mut token);
                    state = LexerState::LineComment;
                }
                '-' if chars.peek() == Some(&'-') => {
                    push_token(&mut tokens, &mut token);
                    chars.next();
                    state = LexerState::LineComment;
                }
                '/' if chars.peek() == Some(&'*') => {
                    push_token(&mut tokens, &mut token);
                    chars.next();
                    state = LexerState::BlockComment;
                }
                value if value.is_ascii_alphanumeric() || value == '_' => {
                    token.push(value.to_ascii_uppercase());
                }
                _ => push_token(&mut tokens, &mut token),
            },
            LexerState::SingleQuote | LexerState::DoubleQuote | LexerState::Backtick => {
                let delimiter = match state {
                    LexerState::SingleQuote => '\'',
                    LexerState::DoubleQuote => '"',
                    LexerState::Backtick => '`',
                    _ => unreachable!(),
                };
                if character == '\\' {
                    chars.next();
                } else if character == delimiter {
                    if chars.peek() == Some(&delimiter) {
                        chars.next();
                    } else {
                        state = LexerState::Normal;
                    }
                }
            }
            LexerState::LineComment if character == '\n' => state = LexerState::Normal,
            LexerState::BlockComment if character == '*' && chars.peek() == Some(&'/') => {
                chars.next();
                state = LexerState::Normal;
            }
            LexerState::LineComment | LexerState::BlockComment => {}
        }
    }
    push_token(&mut tokens, &mut token);
    tokens
}

fn push_token(tokens: &mut Vec<String>, token: &mut String) {
    if !token.is_empty() {
        tokens.push(std::mem::take(token));
    }
}

fn risk_weight(risk: &RiskLevel) -> u8 {
    match risk {
        RiskLevel::Safe => 0,
        RiskLevel::Review => 1,
        RiskLevel::Destructive => 2,
    }
}

#[derive(Clone, Copy)]
enum LexerState {
    Normal,
    SingleQuote,
    DoubleQuote,
    Backtick,
    LineComment,
    BlockComment,
}

fn split_statements(sql: &str) -> Vec<String> {
    let mut statements = Vec::new();
    let mut current = String::new();
    let mut chars = sql.chars().peekable();
    let mut state = LexerState::Normal;
    while let Some(character) = chars.next() {
        current.push(character);
        match state {
            LexerState::Normal => match character {
                '\'' => state = LexerState::SingleQuote,
                '"' => state = LexerState::DoubleQuote,
                '`' => state = LexerState::Backtick,
                '#' => state = LexerState::LineComment,
                '-' if chars.peek() == Some(&'-') => {
                    current.push(chars.next().expect("peeked character"));
                    state = LexerState::LineComment;
                }
                '/' if chars.peek() == Some(&'*') => {
                    current.push(chars.next().expect("peeked character"));
                    state = LexerState::BlockComment;
                }
                ';' => push_statement(&mut statements, &mut current),
                _ => {}
            },
            LexerState::SingleQuote | LexerState::DoubleQuote | LexerState::Backtick => {
                let delimiter = match state {
                    LexerState::SingleQuote => '\'',
                    LexerState::DoubleQuote => '"',
                    LexerState::Backtick => '`',
                    _ => unreachable!(),
                };
                if character == '\\' {
                    if let Some(escaped) = chars.next() {
                        current.push(escaped);
                    }
                } else if character == delimiter {
                    if chars.peek() == Some(&delimiter) {
                        current.push(chars.next().expect("peeked character"));
                    } else {
                        state = LexerState::Normal;
                    }
                }
            }
            LexerState::LineComment if character == '\n' => state = LexerState::Normal,
            LexerState::BlockComment if character == '*' && chars.peek() == Some(&'/') => {
                current.push(chars.next().expect("peeked character"));
                state = LexerState::Normal;
            }
            LexerState::LineComment | LexerState::BlockComment => {}
        }
    }
    push_statement(&mut statements, &mut current);
    statements
}

fn push_statement(statements: &mut Vec<String>, current: &mut String) {
    if !strip_leading_trivia(current)
        .trim_matches(';')
        .trim()
        .is_empty()
    {
        statements.push(std::mem::take(current));
    } else {
        current.clear();
    }
}

fn strip_leading_trivia(mut sql: &str) -> &str {
    loop {
        sql = sql.trim_start();
        if let Some(rest) = sql.strip_prefix("--") {
            sql = rest.split_once('\n').map_or("", |(_, rest)| rest);
            continue;
        }
        if let Some(rest) = sql.strip_prefix('#') {
            sql = rest.split_once('\n').map_or("", |(_, rest)| rest);
            continue;
        }
        if let Some(rest) = sql.strip_prefix("/*") {
            sql = rest.split_once("*/").map_or("", |(_, rest)| rest);
            continue;
        }
        return sql;
    }
}

fn contains_keyword(sql: &str, keyword: &str) -> bool {
    sql.split(|c: char| !c.is_ascii_alphanumeric() && c != '_')
        .any(|part| part == keyword)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn comments_do_not_hide_destructive_statement() {
        let assessment = assess_sql("-- cleanup\nDELETE FROM users");
        assert_eq!(assessment.risk, RiskLevel::Destructive);
        assert!(assessment.requires_confirmation);
    }

    #[test]
    fn read_only_statements_are_safe() {
        assert_eq!(assess_sql(" /* hint */ SELECT 1").risk, RiskLevel::Safe);
    }

    #[test]
    fn later_destructive_statement_is_not_hidden() {
        let assessment = assess_sql("SELECT ';' AS value; DROP TABLE users;");
        assert_eq!(assessment.statement_kind, "MULTI_STATEMENT");
        assert_eq!(assessment.risk, RiskLevel::Destructive);
        assert!(assessment.requires_confirmation);
    }

    #[test]
    fn semicolons_in_comments_and_strings_do_not_split_statements() {
        let assessment = assess_sql("-- ; ignored\nSELECT 'a;b' AS value /* ; */;");
        assert_eq!(assessment.risk, RiskLevel::Safe);
    }

    #[test]
    fn cte_is_conservative_until_parser_classifies_its_body() {
        let assessment = assess_sql("WITH ids AS (SELECT id FROM users) SELECT * FROM ids");
        assert_eq!(assessment.risk, RiskLevel::Review);
        assert!(assessment.requires_confirmation);
    }

    #[test]
    fn mysql_select_side_effects_require_confirmation() {
        for sql in [
            "SELECT * INTO OUTFILE '/tmp/export' FROM users",
            "SELECT * FROM users FOR UPDATE",
            "SELECT GET_LOCK('maintenance', 1)",
            "EXPLAIN ANALYZE SELECT * FROM users",
        ] {
            let assessment = assess_sql(sql);
            assert_eq!(assessment.risk, RiskLevel::Review, "{sql}");
            assert!(assessment.requires_confirmation, "{sql}");
        }
    }

    #[test]
    fn side_effect_words_inside_literals_and_comments_are_ignored() {
        assert_eq!(
            assess_sql("SELECT 'INTO OUTFILE', value FROM users -- FOR UPDATE").risk,
            RiskLevel::Safe
        );
        assert!(assess_sql("SELECT/* comment */SLEEP(1)").requires_confirmation);
    }
}
