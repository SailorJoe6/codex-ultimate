use codex_protocol::custom_commands::CustomCommand;

#[derive(Debug, Clone, PartialEq)]
pub struct CustomCommandExpansion {
    pub text: String,
    pub command: CustomCommand,
}

fn parse_slash_name(text: &str) -> Option<(&str, &str)> {
    let stripped = text.strip_prefix('/')?;
    let mut name_end = stripped.len();
    for (idx, ch) in stripped.char_indices() {
        if ch.is_whitespace() {
            name_end = idx;
            break;
        }
    }
    let name = &stripped[..name_end];
    if name.is_empty() {
        return None;
    }
    let rest_untrimmed = &stripped[name_end..];
    let rest = rest_untrimmed.trim_start();
    Some((name, rest))
}

fn parse_positional_args(rest: &str) -> Vec<String> {
    shlex::split(rest).unwrap_or_else(|| rest.split_whitespace().map(ToString::to_string).collect())
}

fn expand_placeholders(content: &str, args: &[String]) -> String {
    let mut out = String::with_capacity(content.len());
    let mut i = 0;
    while let Some(off) = content[i..].find('$') {
        let j = i + off;
        out.push_str(&content[i..j]);
        let rest = &content[j..];
        let bytes = rest.as_bytes();
        if bytes.len() >= 2 {
            match bytes[1] {
                b'$' => {
                    out.push_str("$$");
                    i = j + 2;
                    continue;
                }
                b'1'..=b'9' => {
                    let idx = (bytes[1] - b'1') as usize;
                    if let Some(arg) = args.get(idx) {
                        out.push_str(arg);
                    }
                    i = j + 2;
                    continue;
                }
                _ => {}
            }
        }
        if rest.len() > "ARGUMENTS".len() && rest[1..].starts_with("ARGUMENTS") {
            if !args.is_empty() {
                out.push_str(&args.join(" "));
            }
            i = j + 1 + "ARGUMENTS".len();
            continue;
        }
        out.push('$');
        i = j + 1;
    }
    out.push_str(&content[i..]);
    out
}

pub fn expand_custom_command(
    text: &str,
    commands: &[CustomCommand],
) -> Option<CustomCommandExpansion> {
    let trimmed = text.trim_start();
    let (name, rest) = parse_slash_name(trimmed)?;
    let command = commands.iter().find(|command| command.name == name)?;
    let args = parse_positional_args(rest);
    let text = expand_placeholders(&command.content, &args);
    Some(CustomCommandExpansion {
        text,
        command: command.clone(),
    })
}

pub fn expand_custom_command_text(text: &str, commands: &[CustomCommand]) -> Option<String> {
    expand_custom_command(text, commands).map(|expansion| expansion.text)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn command(name: &str, content: &str) -> CustomCommand {
        CustomCommand {
            name: name.to_string(),
            path: "dummy".into(),
            content: content.to_string(),
            description: None,
            argument_hint: None,
            allowed_tools: None,
            model: None,
            disable_model_invocation: None,
            scope: codex_protocol::custom_commands::CustomCommandScope::User,
            scope_subdir: None,
        }
    }

    #[test]
    fn expands_positional_and_arguments() {
        let commands = vec![command(
            "deploy",
            "First:$1 Second:$2 All:$ARGUMENTS End:$9",
        )];
        let expanded = expand_custom_command("/deploy prod us-west", &commands).unwrap();
        assert_eq!(
            expanded.text,
            "First:prod Second:us-west All:prod us-west End:"
        );
        assert_eq!(expanded.command.name, "deploy");
    }

    #[test]
    fn preserves_double_dollar() {
        let commands = vec![command("price", "Cost $$1, token $1")];
        let expanded = expand_custom_command("/price usd", &commands).unwrap();
        assert_eq!(expanded.text, "Cost $$1, token usd");
    }

    #[test]
    fn parses_quoted_args() {
        let commands = vec![command("say", "Args:$ARGUMENTS")];
        let expanded = expand_custom_command("/say \"hello world\" ok", &commands).unwrap();
        assert_eq!(expanded.text, "Args:hello world ok");
    }

    #[test]
    fn ignores_unknown_commands() {
        let commands = vec![command("known", "hi")];
        assert_eq!(expand_custom_command("/nope", &commands), None);
    }
}
