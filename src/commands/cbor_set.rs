use crate::util::{apply_changes, CborKeyWritable};
use cbor_data::{Cbor, CborOwned};
use cborpath::CborPath;
use redis_module::{Context, NextArg, RedisError, RedisResult, RedisString, RedisValue, REDIS_OK};

#[derive(Debug, PartialEq, Eq)]
pub enum SetOptions {
    NotExists,
    AlreadyExists,
    None,
}

pub fn cbor_set(ctx: &Context, args: Vec<RedisString>) -> RedisResult {
    let mut args = args.into_iter().skip(1);

    let key_name = args.next_arg()?;
    let cbor_path = args.next_arg()?;
    let new_value = args.next_arg()?;

    let Ok(cbor_path) = CborPath::from_bytes(cbor_path.as_slice()) else {
        return Err(RedisError::Str("ERR Invalid CBOR path"));
    };

    let Ok(new_value) = Cbor::checked(new_value.as_slice()) else {
        return Err(RedisError::Str("ERR Invalid CBOR value"));
    };

    let mut options = SetOptions::None;

    for s in args {
        match s.try_as_str()? {
            arg if arg.eq_ignore_ascii_case("NX") && options == SetOptions::None => {
                options = SetOptions::NotExists
            }
            arg if arg.eq_ignore_ascii_case("XX") && options == SetOptions::None => {
                options = SetOptions::AlreadyExists
            }
            _ => return Err(RedisError::Str("ERR syntax error")),
        };
    }

    let key = ctx.open_key_writable(&key_name);
    let existing_value = key.get_cbor_value()?;

    match set(existing_value, &cbor_path, new_value, options) {
        SetResult::ErrConditionNotMet => Ok(RedisValue::Null),
        SetResult::ErrExpectedRoot => Err(RedisError::Str(
            "ERR new CBOR documents must be created with a root path",
        )),
        SetResult::Updated(new_value) => {
            key.set_cbor_value(new_value)?;
            apply_changes(ctx, "cbor.set", &key_name)?;
            REDIS_OK
        }
        SetResult::NoMatch => REDIS_OK,
    }
}

#[derive(Debug, PartialEq, Eq)]
enum SetResult {
    ErrConditionNotMet,
    ErrExpectedRoot,
    Updated(CborOwned),
    NoMatch,
}

fn set(
    existing_value: Option<&CborOwned>,
    cbor_path: &CborPath,
    new_value: &Cbor,
    options: SetOptions,
) -> SetResult {
    match (existing_value, options) {
        (Some(_), SetOptions::NotExists) => SetResult::ErrConditionNotMet,
        (None, SetOptions::AlreadyExists) => SetResult::ErrConditionNotMet,
        (Some(existing), _) => {
            let new_value = cbor_path.set(existing, new_value);
            if let Some(new_value) = new_value {
                SetResult::Updated(new_value)
            } else {
                SetResult::NoMatch
            }
        }
        (None, _) => {
            if cbor_path.is_root() {
                SetResult::Updated(new_value.to_owned())
            } else {
                SetResult::ErrExpectedRoot
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{set, SetOptions, SetResult};
    use crate::util::{cbor_to_diag, diag_to_cbor};
    use cborpath::CborPath;

    #[test]
    fn not_exists() {
        let result = set(
            Some(&diag_to_cbor("12")),
            &CborPath::root(),
            &diag_to_cbor("13"),
            SetOptions::NotExists,
        );

        assert_eq!(SetResult::ErrConditionNotMet, result);

        let result = set(
            None,
            &CborPath::root(),
            &diag_to_cbor("13"),
            SetOptions::NotExists,
        );

        assert!(matches!(result, SetResult::Updated(cbor) if cbor_to_diag(&cbor) == "13"));
    }

    #[test]
    fn already_exists() {
        let result = set(
            None,
            &CborPath::root(),
            &diag_to_cbor("13"),
            SetOptions::AlreadyExists,
        );
        assert_eq!(SetResult::ErrConditionNotMet, result);

        let result = set(
            Some(&diag_to_cbor("12")),
            &CborPath::root(),
            &diag_to_cbor("13"),
            SetOptions::AlreadyExists,
        );
        assert!(matches!(result, SetResult::Updated(cbor) if cbor_to_diag(&cbor) == "13"));
    }

    #[test]
    fn replace() {
        let result = set(
            Some(&diag_to_cbor("[1,2,3]")),
            &CborPath::builder().index(2).build(),
            &diag_to_cbor("4"),
            SetOptions::AlreadyExists,
        );
        assert!(matches!(result, SetResult::Updated(cbor) if cbor_to_diag(&cbor) == "[1,2,4]"));
    }
}
