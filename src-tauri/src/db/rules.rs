//! Storage of the import rules.

use rusqlite::{Connection, params};

use super::{Error, Result, Vault};
use crate::import::rules::{Rule, Source, Target};

impl Vault {
    pub fn import_rules(&self) -> Result<Vec<Rule>> {
        let mut stmt = self.conn.prepare(
            "SELECT name, enabled, source, lines, pattern, target, map, strip FROM import_rules ORDER BY position, id",
        )?;
        let rows = stmt.query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, bool>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, u32>(3)?,
                r.get::<_, String>(4)?,
                r.get::<_, String>(5)?,
                r.get::<_, String>(6)?,
                r.get::<_, bool>(7)?,
            ))
        })?;
        rows.map(|row| {
            let (name, enabled, source, lines, pattern, target, map, strip) = row?;
            let corrupt = |what: &str| Error::InvalidRule(format!("rule '{name}': stored {what} is not valid"));
            Ok(Rule {
                source: Source::parse(&source).ok_or_else(|| corrupt("source"))?,
                target: Target::try_from(target).map_err(|_| corrupt("target"))?,
                map: serde_json::from_str(&map).map_err(|_| corrupt("value map"))?,
                name,
                enabled,
                lines,
                pattern,
                strip,
            })
        })
        .collect()
    }

    /// Replaces all rules, in order. Every rule must compile, or nothing is saved.
    pub fn set_import_rules(&mut self, rules: &[Rule]) -> Result<()> {
        for rule in rules {
            rule.compile().map_err(Error::InvalidRule)?;
        }
        let tx = self.conn.transaction()?;
        tx.execute("DELETE FROM import_rules", [])?;
        write_rules(&tx, rules)?;
        tx.commit()?;
        Ok(())
    }
}

pub(super) fn write_rules(conn: &Connection, rules: &[Rule]) -> Result<()> {
    let mut stmt = conn.prepare(
        "INSERT INTO import_rules(position, name, enabled, source, lines, pattern, target, map, strip)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
    )?;
    for (i, rule) in rules.iter().enumerate() {
        let map = serde_json::to_string(&rule.map).expect("pairs of strings serialize");
        stmt.execute(params![
            i as i64,
            rule.name.trim(),
            rule.enabled,
            rule.source.as_str(),
            rule.lines,
            rule.pattern,
            String::from(rule.target.clone()),
            map,
            rule.strip,
        ])?;
    }
    Ok(())
}
