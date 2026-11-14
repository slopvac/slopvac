use semver::Version;
use toml::{Table, Value};

use super::model::{
    Digest, KeyId, LockError, LockedPack, Lockfile, NotBefore, PackId, PackKind, Signature, Source,
    ValueError,
};

fn err(pointer: &str, message: impl Into<String>) -> LockError {
    LockError {
        pointer: pointer.to_owned(),
        message: message.into(),
    }
}

/// A TOML table with its JSON Pointer, read under the unknown-key policy.
struct Obj<'a> {
    table: &'a Table,
    pointer: String,
}

impl<'a> Obj<'a> {
    fn new(table: &'a Table, pointer: String, allowed: &[&str]) -> Result<Self, LockError> {
        if let Some(key) = table
            .keys()
            .find(|k| !k.starts_with("x-") && !allowed.contains(&k.as_str()))
        {
            return Err(err(&pointer, format!("unknown key `{key}`")));
        }
        Ok(Self { table, pointer })
    }

    fn at(&self, key: &str) -> String {
        format!("{}/{key}", self.pointer)
    }

    fn required(&self, key: &str) -> Result<&'a Value, LockError> {
        self.table
            .get(key)
            .ok_or_else(|| err(&self.pointer, format!("missing required key `{key}`")))
    }

    fn string(&self, key: &str) -> Result<&'a str, LockError> {
        self.required(key)?
            .as_str()
            .ok_or_else(|| err(&self.at(key), "must be a string"))
    }

    fn parsed<T: std::str::FromStr<Err = ValueError>>(&self, key: &str) -> Result<T, LockError> {
        self.string(key)?
            .parse()
            .map_err(|e: ValueError| err(&self.at(key), e.to_string()))
    }
}

pub(super) fn parse(text: &str) -> Result<Lockfile, LockError> {
    let table: Table = toml::from_str(text).map_err(|e| err("", format!("invalid TOML: {e}")))?;
    let root = Obj::new(&table, String::new(), &["schema_version", "packs"])?;
    match root.required("schema_version")? {
        Value::Integer(3) => {}
        _ => return Err(err("/schema_version", "must be the integer 3")),
    }
    let items = root
        .required("packs")?
        .as_array()
        .ok_or_else(|| err("/packs", "must be an array of tables"))?;
    let packs = items
        .iter()
        .enumerate()
        .map(|(i, item)| {
            let pointer = format!("/packs/{i}");
            let table = item
                .as_table()
                .ok_or_else(|| err(&pointer, "must be a table"))?;
            pack(table, pointer)
        })
        .collect::<Result<Vec<_>, _>>()?;
    Lockfile::new(packs)
}

fn pack(table: &Table, pointer: String) -> Result<LockedPack, LockError> {
    let obj = Obj::new(
        table,
        pointer,
        &[
            "id",
            "kind",
            "version",
            "slopvac_api",
            "source",
            "digest",
            "signature",
            "requires",
        ],
    )?;
    // Report a missing required key before inspecting any value.
    for key in ["id", "kind", "version", "slopvac_api", "source", "digest"] {
        obj.required(key)?;
    }
    let version_text = obj.string("version")?;
    let version = Version::parse(version_text)
        .ok()
        .filter(|v| v.build.is_empty())
        .ok_or_else(|| {
            err(
                &obj.at("version"),
                format!("`{version_text}` is not a semantic version without build metadata"),
            )
        })?;
    let slopvac_api = obj
        .required("slopvac_api")?
        .as_integer()
        .and_then(|n| u32::try_from(n).ok())
        .filter(|&n| n >= 1)
        .ok_or_else(|| err(&obj.at("slopvac_api"), "must be an integer of at least 1"))?;
    Ok(LockedPack {
        id: obj.parsed::<PackId>("id")?,
        kind: obj.parsed::<PackKind>("kind")?,
        version,
        slopvac_api,
        source: obj.parsed::<Source>("source")?,
        digest: obj.parsed::<Digest>("digest")?,
        signature: obj
            .table
            .get("signature")
            .map(|v| signature(v, obj.at("signature")))
            .transpose()?,
        requires: requires(&obj)?,
    })
}

fn signature(value: &Value, pointer: String) -> Result<Signature, LockError> {
    let table = value
        .as_table()
        .ok_or_else(|| err(&pointer, "must be a table"))?;
    let obj = Obj::new(table, pointer, &["key_id", "not_before"])?;
    for key in ["key_id", "not_before"] {
        obj.required(key)?;
    }
    Ok(Signature {
        key_id: obj.parsed::<KeyId>("key_id")?,
        not_before: obj.parsed::<NotBefore>("not_before")?,
    })
}

fn requires(obj: &Obj<'_>) -> Result<Vec<PackId>, LockError> {
    let Some(value) = obj.table.get("requires") else {
        return Ok(Vec::new());
    };
    let pointer = obj.at("requires");
    let items = value
        .as_array()
        .ok_or_else(|| err(&pointer, "must be an array of pack ids"))?;
    let mut ids = Vec::with_capacity(items.len());
    for (i, item) in items.iter().enumerate() {
        let at = format!("{pointer}/{i}");
        let id: PackId = item
            .as_str()
            .ok_or_else(|| err(&at, "must be a string"))?
            .parse()
            .map_err(|e: ValueError| err(&at, e.to_string()))?;
        if ids.contains(&id) {
            return Err(err(&at, format!("`{id}` is listed twice")));
        }
        ids.push(id);
    }
    Ok(ids)
}
