//! `case`: one name in every case code uses. Words are found as heck finds
//! them (the boundaries cargo and clap use): at spaces, `_ - . /`, and changes
//! of case (`userID` is user, ID; `HTTPServer` is HTTP, Server). Each line of
//! the input is a name of its own.

use heck::{
    ToKebabCase, ToLowerCamelCase, ToShoutySnakeCase, ToSnakeCase, ToTitleCase, ToTrainCase,
    ToUpperCamelCase,
};
use toolwing_core::{
    Category, Cli, Ctx, Error, Field, InputKind, Meta, Output, Param, ParamKind, Tool, Value,
    Values, en,
};

pub struct Convert;

static META: Meta = Meta {
    id: "case",
    group: "case",
    action: "",
    action_flag: None,
    title: en!("Case"),
    summary: en!("One name in every case code uses: camelCase, snake_case, kebab-case…"),
    category: Category::Text,
    keywords: &[
        "case", "camel", "snake", "kebab", "pascal", "constant", "title",
    ],
    input: InputKind::Text { trim_newline: true },
    streams: false,
    reference: None,
};

/// (value, label): the label is the style written in itself.
const STYLES: &[(&str, &str)] = &[
    ("camel", "camelCase"),
    ("pascal", "PascalCase"),
    ("snake", "snake_case"),
    ("constant", "CONSTANT_CASE"),
    ("kebab", "kebab-case"),
    ("train", "Train-Case"),
    ("dot", "dot.case"),
    ("title", "Title Case"),
    ("sentence", "Sentence case"),
];

static PARAMS: &[Param] = &[Param {
    key: "to",
    label: en!("Case"),
    help: en!("The case to write; every case when left out"),
    kind: ParamKind::Choice {
        options: &[
            ("camel", "camelCase"),
            ("pascal", "PascalCase"),
            ("snake", "snake_case"),
            ("constant", "CONSTANT_CASE"),
            ("kebab", "kebab-case"),
            ("train", "Train-Case"),
            ("dot", "dot.case"),
            ("title", "Title Case"),
            ("sentence", "Sentence case"),
        ],
        flags: true,
        unset: Some("All"),
    },
    default: Value::Text(String::new()),
    cli: Cli {
        long: "to",
        short: None,
    },
}];

/// `name` in `style`; `None` for a style there is not.
pub fn convert(name: &str, style: &str) -> Option<String> {
    Some(match style {
        "camel" => name.to_lower_camel_case(),
        "pascal" => name.to_upper_camel_case(),
        "snake" => name.to_snake_case(),
        "constant" => name.to_shouty_snake_case(),
        "kebab" => name.to_kebab_case(),
        "train" => name.to_train_case(),
        "dot" => name.to_snake_case().replace('_', "."),
        "title" => name.to_title_case(),
        "sentence" => {
            let words = name.to_snake_case().replace('_', " ");
            let mut chars = words.chars();
            match chars.next() {
                Some(first) => first.to_uppercase().chain(chars).collect(),
                None => String::new(),
            }
        }
        _ => return None,
    })
}

fn each_line(text: &str, style: &str) -> String {
    text.split('\n')
        .map(|line| convert(line, style).unwrap_or_default())
        .collect::<Vec<_>>()
        .join("\n")
}

impl Tool for Convert {
    fn meta(&self) -> &'static Meta {
        &META
    }

    fn params(&self) -> &'static [Param] {
        PARAMS
    }

    fn run(&self, _ctx: &Ctx, input: &[u8], values: &Values) -> Result<Output, Error> {
        let text = std::str::from_utf8(input).map_err(|_| Error::input("input is not UTF-8"))?;
        let to = values.text(PARAMS, "to");
        if to.is_empty() {
            return Ok(Output::Fields(
                STYLES
                    .iter()
                    .map(|(key, label)| Field::new(*key, *label, each_line(text, key)))
                    .collect(),
            ));
        }
        if convert("", &to).is_none() {
            return Err(Error::usage(format!("no case named {to}")));
        }
        Ok(Output::Text(each_line(text, &to)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_case_of_a_name() {
        let name = "user account ID";
        let want = [
            ("camel", "userAccountId"),
            ("pascal", "UserAccountId"),
            ("snake", "user_account_id"),
            ("constant", "USER_ACCOUNT_ID"),
            ("kebab", "user-account-id"),
            ("train", "User-Account-Id"),
            ("dot", "user.account.id"),
            ("title", "User Account Id"),
            ("sentence", "User account id"),
        ];
        for (style, out) in want {
            assert_eq!(convert(name, style).as_deref(), Some(out), "{style}");
        }
    }

    #[test]
    fn words_are_found_at_case_changes() {
        assert_eq!(
            convert("HTTPServerError", "snake").as_deref(),
            Some("http_server_error")
        );
        assert_eq!(convert("userID", "kebab").as_deref(), Some("user-id"));
        assert_eq!(
            convert("api_v2-response.body", "camel").as_deref(),
            Some("apiV2ResponseBody")
        );
        assert_eq!(convert("", "sentence").as_deref(), Some(""));
    }

    #[test]
    fn all_cases_or_one_and_each_line_apart() {
        let run = |input: &str, to: &str| {
            let mut v = Values::new();
            v.set("to", Value::Text(to.into()));
            Convert.run(&Ctx::new(), input.as_bytes(), &v)
        };
        match run("a b", "") {
            Ok(Output::Fields(f)) => {
                assert_eq!(f.len(), STYLES.len());
                assert_eq!((f[2].key.as_str(), f[2].value.as_str()), ("snake", "a_b"));
            }
            other => panic!("{other:?}"),
        }
        assert_eq!(
            run("one two\nthree four", "kebab"),
            Ok(Output::Text("one-two\nthree-four".into()))
        );
        assert!(run("x", "nope").is_err());
    }
}
