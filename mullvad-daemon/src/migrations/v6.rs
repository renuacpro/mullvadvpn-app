use super::Result;
use mullvad_types::{
    //relay_constraints::Constraint,
    settings::SettingsVersion
};

// ======================================================
// Section for vendoring types and values that
// this settings version depend on. See `mod.rs`.

// ...

// ======================================================

/// TODO: Write in this documentation how the settings format changed, what the migration does.
pub fn migrate(settings: &mut serde_json::Value) -> Result<()> {
    if !version_matches(settings) {
        return Ok(());
    }

    //log::info!("Migrating settings format to V7");

    // TODO: Insert migration code here

    //settings["settings_version"] = serde_json::json!(SettingsVersion::V7);

    Ok(())
}

fn version_matches(settings: &mut serde_json::Value) -> bool {
  settings
      .get("settings_version")
      .map(|version| version == SettingsVersion::V6 as u64)
      .unwrap_or(false)
}

#[cfg(test)]
mod test {
    // use super::{migrate, version_matches};
    // use serde_json;

    // TODO: Implement tests. Look at other migration modules for inspiration.
}
