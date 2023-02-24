#[cfg(debug_assertions)]
use crossterm::event;
#[cfg(debug_assertions)]
use std::env;

use args::Arguments;
use atty::Stream;
use clap::Parser;
use ntreg::{registry_key::RegistryKey, registry_value::RegistryValueData, registry_value::RegistryValue};
use ntstatuserror::NtStatusErrorKind;
use std::{io::{self, Read}, process::exit};

use crate::config::RegistryConfig;

mod args;
#[cfg(onecore)]
mod bcrypt;
mod config;

const EXIT_SUCCESS: i32 = 0;
const EXIT_INVALID_PARAMETER: i32 = 1;
const EXIT_INVALID_INPUT: i32 = 2;
const EXIT_REGISTRY_ERROR: i32 = 3;
const EXIT_NOT_IN_DESIRED_STATE: i32 = 4;
const EXIT_JSON_SERIALIZATION_FAILED: i32 = 5;

fn main() {
    #[cfg(debug_assertions)]
    check_debug();

    let args = Arguments::parse();
    let input: Option<String> = if atty::is(Stream::Stdin) {
        None
    } else {
        let mut buffer: Vec<u8> = Vec::new();
        io::stdin().read_to_end(&mut buffer).unwrap();
        let input = match String::from_utf8(buffer) {
            Ok(input) => input,
            Err(e) => {
                eprintln!("Invalid UTF-8 sequence: {}", e);
                exit(EXIT_INVALID_INPUT);
            }
        };
        Some(input)
    };
    
    let mut config: RegistryConfig = Default::default();
    // check if input is valid for subcommand
    match args.subcommand {
        args::SubCommand::Config { subcommand: _ } => {
            match input {
                Some(input) => {
                    config = match serde_json::from_str(&input) {
                        Ok(config) => config,
                        Err(err) => {
                            eprintln!("Error JSON does not match schema: {}", err);
                            exit(EXIT_INVALID_INPUT);
                        }
                    };
                },
                None => {
                    eprintln!("Error: Input JSON via STDIN is required for config subcommand.");
                    exit(EXIT_INVALID_PARAMETER);
                }
            }
        }
        _ => {
            if input.is_some() {
                eprintln!("Error: Input JSON via STDIN is only valid for config subcommand.");
                exit(EXIT_INVALID_INPUT);
            }
        }
    }

    match args.subcommand {
        args::SubCommand::Query { key_path, value_name, recurse } => {
            eprintln!("Get key_path: {}, value_name: {:?}, recurse: {}", key_path, value_name, recurse);
        },
        args::SubCommand::Set { key_path, value } => {
            eprintln!("Set key_path: {}, value: {}", key_path, value);
        },
        args::SubCommand::Test => {
            eprintln!("Test");
        },
        args::SubCommand::Remove { key_path, value_name, recurse } => {
            eprintln!("Remove key_path: {}, value_name: {:?}, recurse: {}", key_path, value_name, recurse);
        },
        args::SubCommand::Find { key_path, find, recurse, keys_only, values_only } => {
            eprintln!("Find key_path: {}, find: {}, recurse: {:?}, keys_only: {:?}, values_only: {:?}", key_path, find, recurse, keys_only, values_only);
        },
        args::SubCommand::Config { subcommand } => {
            let json: String;
            let in_desired_state: bool;
            validate_config(&config);
            if config.ensure.is_none() {
                config.ensure = Some(config::EnsureKind::Present);
            }

            match subcommand {
                args::ConfigSubCommand::Get => {
                    println!("{}", config_get(&config));
                    exit(EXIT_SUCCESS);
                },
                args::ConfigSubCommand::Set => {
                    (json, in_desired_state) = config_set(&config);
                },
                args::ConfigSubCommand::Test => {
                    (json, in_desired_state) = config_test(&config);
                },
            }

            if json.is_empty() {
                exit(EXIT_JSON_SERIALIZATION_FAILED);
            }

            println!("{}", json);
            if !in_desired_state {
                exit(EXIT_NOT_IN_DESIRED_STATE);
            }
        }
    }

    exit(EXIT_SUCCESS);
}

#[cfg(debug_assertions)]
fn check_debug() {
    if env::var("DEBUG_REGISTRY").is_ok() {
        eprintln!("attach debugger");
        loop {
            let event = event::read().unwrap();
            match event {
                event::Event::Key(_key) => {
                    break;
                }
                _ => {
                    eprintln!("Unexpected event: {:?}", event);
                    continue;
                }
            }
        }
    }
}

fn config_get(config: &RegistryConfig) -> String {
    let reg_key = match RegistryKey::new(config.key_path.as_str()) {
        Ok(reg_key) => reg_key,
        Err(err) => {
            eprintln!("Error: {}", err);
            exit(EXIT_REGISTRY_ERROR);
        }
    };

    let mut reg_result = RegistryConfig {
        key_path: config.key_path.clone(),
        value_name: None,
        value_data: None,
        ensure: None,
        clobber: None,
    };

    if config.value_name.is_some() {
        let reg_value = match reg_key.get_value(config.value_name.as_ref().unwrap().as_str()) {
            Ok(reg_value) => reg_value,
            Err(err) => {
                eprintln!("Error: {}", err);
                exit(EXIT_REGISTRY_ERROR);
            }
        };

        reg_result.value_name = Some(reg_value.name);
        reg_result.value_data = Some(convert_ntreg_data(&reg_value.data));
    }

    match serde_json::to_string(&reg_result) {
        Ok(reg_json) => reg_json,
        Err(err) => {
            eprintln!("Error: {}", err);
            exit(EXIT_REGISTRY_ERROR);
        }
    }
}

fn config_set(config: &RegistryConfig) -> (String, bool) {
    let mut reg_result: RegistryConfig = Default::default();
    reg_result.key_path = config.key_path.clone();
    let in_desired_state = true;

    let reg_key: RegistryKey;
    match &config.value_name {
        None => {
            match config.ensure.as_ref().unwrap() {
                config::EnsureKind::Present => {
                    open_or_create_key(&config.key_path);
                },
                config::EnsureKind::Absent => {
                    remove_key(&config.key_path);
                },
            }
        },
        Some(value_name) => {
            reg_result.value_name = Some(value_name.clone());
            match &config.ensure {
                Some(config::EnsureKind::Present) | None => {
                    reg_key = open_or_create_key(&config.key_path);
                    match config.value_data.as_ref() {
                        Some(value_data) => {
                            reg_result.value_data = Some(value_data.clone());
                            match reg_key.set_value(value_name, &convert_configreg_data(value_data)) {
                                Ok(_) => {},
                                Err(err) => {
                                    eprintln!("Error: {}", err);
                                    exit(EXIT_REGISTRY_ERROR);
                                }
                            }
                        },
                        None => {
                            // just verify that the value exists
                            match reg_key.get_value(value_name) {
                                Ok(_reg_value) => {},
                                Err(err) => {
                                    match err.status {
                                        NtStatusErrorKind::ObjectNameNotFound => {
                                            match reg_key.set_value(value_name, &ntreg::registry_value::RegistryValueData::None) {
                                                Ok(_) => {},
                                                Err(err) => {
                                                    eprintln!("Error: {}", err);
                                                    exit(EXIT_REGISTRY_ERROR);
                                                }
                                            }
                                        },
                                        _ => {
                                            eprintln!("Error: {}", err);
                                            exit(EXIT_REGISTRY_ERROR);
                                        },
                                    }
                                }
                            }
                        }
                    }
                },
                Some(config::EnsureKind::Absent) => {
                    reg_key = open_or_create_key(&config.key_path);
                    match reg_key.delete_value(value_name) {
                        Ok(_) => {},
                        Err(err) => {
                            match err.status {
                                NtStatusErrorKind::ObjectNameNotFound => {},
                                _ => {
                                    eprintln!("Error: {}", err);
                                    exit(EXIT_REGISTRY_ERROR);
                                },
                            }
                        }
                    }
                },
            }
        }
    }

    let reg_json = match serde_json::to_string(&reg_result) {
        Ok(reg_json) => reg_json,
        Err(err) => {
            eprintln!("Error: {}", err);
            exit(EXIT_REGISTRY_ERROR);
        }
    };

    (reg_json, in_desired_state)
}

fn get_parent_key_path(key_path: &str) -> &str {
    match key_path.rfind('\\') {
        Some(index) => &key_path[..index],
        None => {
            eprintln!("Error: Invalid key path: {}", key_path);
            exit(EXIT_INVALID_INPUT);
        }
    }
}

fn remove_key(key_path: &str) {
    match RegistryKey::new(key_path) {
        Ok(key) => {
            match key.delete(true) {
                Ok(_) => {},
                Err(err) => {
                    eprintln!("Error: {}", err);
                    exit(EXIT_REGISTRY_ERROR);
                }
            }
        },
        Err(err) => {
            match err.status {
                NtStatusErrorKind::ObjectNameNotFound => {},
                _ => {
                    eprintln!("Error: {}", err);
                    exit(EXIT_REGISTRY_ERROR);
                }
            }
        }
    }
}

fn open_or_create_key(key_path: &str) -> RegistryKey {
    let reg_key: RegistryKey;
    match RegistryKey::new(key_path) {
        Ok(key) => {
            reg_key = key;
        },
        Err(err) => {
            match err.status {
                NtStatusErrorKind::ObjectNameNotFound =>{
                    // need to handle case like `HKLM\1\2\3` where neither `1` nor `2` exist
                    // so we need to find the top most parent that currently exists and then create the necessary subkeys in order
                    let (parent_key, subkeys) = get_valid_parent_key_and_subkeys(key_path);
                    let mut current_key = parent_key;
                    for subkey in subkeys {
                        match current_key.create_key(subkey) {
                            Ok(key) => {
                                current_key = key;
                            },
                            Err(err) => {
                                eprintln!("Error: {}", err);
                                exit(EXIT_REGISTRY_ERROR);
                            }
                        }
                    }
                    reg_key = current_key;
                },
                _ => {
                    eprintln!("Error: {}", err);
                    exit(EXIT_REGISTRY_ERROR);
                }
            }
        }
    }

    reg_key
}

fn get_valid_parent_key_and_subkeys(key_path: &str) -> (RegistryKey, Vec<&str>) {
    let parent_key: RegistryKey;
    let mut subkeys: Vec<&str> = Vec::new();
    let parent_key_path = get_parent_key_path(key_path);
    let subkey_name = &key_path[parent_key_path.len() + 1..];
    subkeys.push(subkey_name);
    let mut current_key_path = parent_key_path;

    loop {
        match RegistryKey::new(current_key_path) {
            Ok(key) => {
                parent_key = key;
                break;
            },
            Err(err) => {
                match err.status {
                    NtStatusErrorKind::ObjectNameNotFound => {
                        let parent_key_path = get_parent_key_path(current_key_path);
                        let subkey_name = &current_key_path[parent_key_path.len() + 1..];
                        subkeys.insert(0, subkey_name);
                        current_key_path = parent_key_path;
                    },
                    _ => {
                        eprintln!("Error: {}", err);
                        exit(EXIT_REGISTRY_ERROR);
                    }
                }
            }
        }
    }

    (parent_key, subkeys)
}

fn validate_config(config: &RegistryConfig) {
    if config.value_data.is_some() && config.value_name.is_none() {
        eprintln!("Error: value_name is required when value_data is specified.");
        exit(EXIT_INVALID_INPUT);
    }
}

fn config_test(config: &RegistryConfig) -> (String, bool) {
    if config.value_name.is_none() {
        test_key(config)
    }
    else {
        test_value(config)
    }
}

fn test_value(config: &RegistryConfig) -> (String, bool) {
    let mut reg_result: RegistryConfig = Default::default();
    let mut in_desired_state = true;

    let reg_key: RegistryKey;
    match RegistryKey::new(config.key_path.as_str()) {
        Ok(key) => {
            reg_key = key;
        },
        Err(err) => {
            match err.status {
                NtStatusErrorKind::ObjectNameNotFound =>{
                    reg_result.key_path = String::new();
                    return (reg_result.to_json(), false);
                },
                _ => {
                    eprintln!("Error: {}", err);
                    exit(EXIT_REGISTRY_ERROR);
                }
            }
        }
    };

    reg_result.key_path = config.key_path.clone();

    let value_name = config.value_name.as_ref().unwrap();
    let mut value_exists = false;
    let reg_value: RegistryValue = match reg_key.get_value(value_name) {
        Ok(value) => {
            value_exists = true;
            reg_result.value_name = Some(value.name.clone());
            reg_result.value_data = Some(convert_ntreg_data(&value.data));
            value
        },
        Err(err) => {
            match err.status {
                NtStatusErrorKind::ObjectNameNotFound => {
                    RegistryValue {
                        key_path: config.key_path.clone(),
                        name : String::new(),
                        data : RegistryValueData::None,
                    }
                },
                _ => {
                    eprintln!("Error: {}", err);
                    exit(EXIT_REGISTRY_ERROR);
                }
            }
        }
    };

    match &config.ensure.as_ref().unwrap() {
        config::EnsureKind::Present => {
            if value_exists {
                in_desired_state = reg_values_are_eq(config, &reg_value);
            }
            else {
                in_desired_state = false;
            }
        },
        config::EnsureKind::Absent => {
            if value_exists {
                in_desired_state = false;
            }
        }
    }

    (reg_result.to_json(), in_desired_state)
}

fn reg_values_are_eq(config: &RegistryConfig, reg_value: &RegistryValue) -> bool {
    let mut in_desired_state = true;

    if !reg_value.name.eq(config.value_name.as_ref().unwrap().as_str()) {
        in_desired_state = false;
    }

    if config.value_data.is_some() && reg_value.data == RegistryValueData::None {
        in_desired_state = false;
    }
    else if config.value_data.is_none() {
        in_desired_state = true;
    }
    else {
        let reg_value_data = convert_ntreg_data(&reg_value.data);
        if reg_value_data != config.value_data.to_owned().unwrap() {
            in_desired_state = false;
        }
    }

    in_desired_state
}

fn test_key(config: &RegistryConfig) -> (String, bool) {
    let mut reg_result: RegistryConfig = Default::default();

    let key_exists;
    match RegistryKey::new(config.key_path.as_str()) {
        Ok( _ ) => {
            key_exists = true;
        },
        Err(err) => {
            match err.status {
                NtStatusErrorKind::ObjectNameNotFound => {
                    key_exists = false;
                },
                _ => {
                    eprintln!("Error: {}", err);
                    exit(EXIT_REGISTRY_ERROR);
                }
            }
        }
    };

    let mut in_desired_state = true;
    match &config.ensure.as_ref().unwrap() {
        config::EnsureKind::Present => {
            if !key_exists {
                reg_result.key_path = String::new();
                in_desired_state = false;
            }
        },
        config::EnsureKind::Absent => {
            if key_exists {
                reg_result.key_path = config.key_path.clone();
                in_desired_state = false;
            }
        }
    }
        
    (reg_result.to_json(), in_desired_state)
}

fn convert_ntreg_data(reg_data: &ntreg::registry_value::RegistryValueData) -> config::RegistryValueData {
    match reg_data {
        RegistryValueData::String(data) => config::RegistryValueData::String(data.clone()),
        RegistryValueData::MultiString(data) => config::RegistryValueData::MultiString(data.clone()),
        RegistryValueData::Binary(data) => config::RegistryValueData::Binary(data.clone()),
        RegistryValueData::DWord(data) => config::RegistryValueData::DWord(*data),
        RegistryValueData::QWord(data) => config::RegistryValueData::QWord(*data),
        RegistryValueData::ExpandString(data) => config::RegistryValueData::ExpandString(data.clone()),
        _ => {
            eprintln!("Error: Unsupported registry value type.");
            exit(EXIT_REGISTRY_ERROR);
        }
    }
}

fn convert_configreg_data(reg_data: &config::RegistryValueData) -> ntreg::registry_value::RegistryValueData {
    match reg_data {
        config::RegistryValueData::String(data) => RegistryValueData::String(data.clone()),
        config::RegistryValueData::MultiString(data) => RegistryValueData::MultiString(data.clone()),
        config::RegistryValueData::Binary(data) => RegistryValueData::Binary(data.clone()),
        config::RegistryValueData::DWord(data) => RegistryValueData::DWord(*data),
        config::RegistryValueData::QWord(data) => RegistryValueData::QWord(*data),
        config::RegistryValueData::ExpandString(data) => RegistryValueData::ExpandString(data.clone()),
    }
}

#[test]
fn test_registry_value_present() {
    let input_json: &str = r#"
    {
        "keyPath": "HKLM\\Software\\Microsoft\\Windows\\CurrentVersion",
        "valueName": "ProgramFilesPath",
        "_ensure": "Present"
    }
    "#;

    let config: RegistryConfig = serde_json::from_str(input_json).unwrap();
    let (json, in_desired_state) = config_test(&config);
    assert!(in_desired_state);
    assert_eq!(json, r#"{"keyPath":"HKLM\\Software\\Microsoft\\Windows\\CurrentVersion","valueName":"ProgramFilesPath","valueData":{"ExpandString":"%ProgramFiles%"}}"#);
}

#[test]
fn test_registry_value_absent() {
    let input_json: &str = r#"
    {
        "keyPath": "HKLM\\Software\\Microsoft\\Windows\\CurrentVersion",
        "valueName": "DoesNotExist",
        "_ensure": "Absent"
    }
    "#;

    let config: RegistryConfig = serde_json::from_str(input_json).unwrap();
    let (json, in_desired_state) = config_test(&config);
    assert!(in_desired_state);
    assert_eq!(json, r#"{"keyPath":"HKLM\\Software\\Microsoft\\Windows\\CurrentVersion"}"#);
}
