use std::collections::{BTreeSet, HashMap, HashSet};
use std::io::Write;
use std::path::Path;
use std::{fs, io};

use red4ext_rs::types::TaggedType;
use red4ext_rs::{
    export_plugin_symbols, log, wcstr, GameApp, Plugin, RttiSystem, SdkEnv, SemVer, StateListener,
    StateType, U16CStr,
};
use redscript_io::{Definition, ScriptBundle};

pub struct DumperPlugin;

impl Plugin for DumperPlugin {
    const AUTHOR: &'static U16CStr = wcstr!("jekky");
    const NAME: &'static U16CStr = wcstr!("redscript-sealed-struct-dumper");
    const VERSION: SemVer = SemVer::new(0, 1, 0);

    fn on_init(env: &SdkEnv) {
        env.add_listener(
            StateType::Running,
            StateListener::default().with_on_enter(on_game_running),
        );
    }
}

export_plugin_symbols!(DumperPlugin);

unsafe extern "C" fn on_game_running(_app: &GameApp) {
    log::info!("Game is running, dumping...");
    if let Err(e) = run() {
        log::error!("Error: {:?}", e);
    }
}

fn run() -> anyhow::Result<()> {
    let rtti = RttiSystem::get();

    let exe_path = std::env::current_exe()?;
    let game_root = exe_path
        .parent()
        .and_then(Path::parent)
        .and_then(Path::parent)
        .ok_or_else(|| anyhow::anyhow!("failed to get game root"))?;

    let data = fs::read(game_root.join("r6/cache/final.redscripts"))?;
    let bundle =
        ScriptBundle::from_bytes(&data).map_err(|e| anyhow::anyhow!("I/O error: {:?}", e))?;

    let mut known_structs = HashMap::new();

    for def in bundle.definitions() {
        match def {
            Definition::Class(class) if class.flags().is_struct() => {
                let name = bundle.get_item(class.name()).ok_or_else(|| {
                    anyhow::anyhow!("failed to get name for struct {}", class.name())
                })?;
                known_structs.insert(name, &**class);
            }
            _ => continue,
        }
    }

    let mut matches = BTreeSet::new();

    for (name, type_) in rtti.type_map() {
        let TaggedType::Class(class) = type_.tagged() else {
            continue;
        };
        if class.base().is_some() || class.flags().is_scripted_struct() {
            continue;
        }

        let scripted_name = rtti.native_to_script_map().get(name).unwrap_or(name);
        let mut current_size = 0;
        let mut contains_padding = false;
        let Some(bundle_struct) = known_structs.get(scripted_name.as_str()) else {
            continue;
        };
        let mut remaining = bundle_struct
            .fields()
            .iter()
            .map(|&idx| {
                let field = bundle
                    .get_item(idx)
                    .ok_or_else(|| anyhow::anyhow!("failed to get field"))?;
                bundle
                    .get_item(field.name())
                    .ok_or_else(|| anyhow::anyhow!("failed to get field name"))
            })
            .collect::<Result<HashSet<_>, _>>()?;

        let mut props = class.properties().iter().copied().collect::<Vec<_>>();
        props.sort_by_key(|prop| prop.value_offset());

        for prop in props {
            let offset = prop.value_offset();
            if offset < current_size {
                log::info!("Propety {} falls behind the current offset", prop.name());
                contains_padding = true;
                break;
            };
            if offset != current_size.next_multiple_of(prop.type_().alignment()) {
                contains_padding = true;
            }
            remaining.remove(&prop.name().as_str());
            current_size = offset + prop.type_().size();
        }

        if remaining.is_empty() && !contains_padding {
            matches.insert(scripted_name.as_str());
        }
    }

    let output = game_root.join("sealed-structs.txt");
    let mut output = io::BufWriter::new(fs::File::create(output)?);
    for name in matches {
        writeln!(output, "{name}")?;
    }

    Ok(())
}
