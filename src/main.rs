use std::env;
use std::fs::{create_dir_all, File};
use std::io::{Write, BufWriter};
use std::path::Path;
use std::process::Command;
use serde::Deserialize;
use walkdir::WalkDir;
use sha3::{Digest, Keccak256};

#[derive(Deserialize)]
struct AbiEntry {
    #[serde(rename = "type")]
    kind: String,
    name: Option<String>,
    inputs: Option<Vec<AbiInput>>,
    // For events, if not specified we assum false.
    #[serde(default)]
    anonymous: bool,
}

#[derive(Deserialize)]
struct AbiInput {
    #[serde(rename = "type")]
    param_type: String,
}

/// Special character cases for .csv: comas, quotes or line breaks, enclose between quotes.
fn escape_csv_field(field: &str) -> String {
    if field.contains(',') || field.contains('"') || field.contains('\n') {
        let escaped = field.replace('"', "\"\"");
        format!("\"{}\"", escaped)
    } else {
        field.to_string()
    }
}

fn main() {
    // Taking args from terminal commands:
    // 1st arg: contract's folder path.
    // 2nd arg (optional): output's folder path.
    let args: Vec<String> = env::args().collect();
    if args.len() < 2 {
        eprintln!("How to use: {} <contracts_folder_oath> [output_folder_path]", args[0]);
        std::process::exit(1);
    }
    let contracts_path = Path::new(&args[1]);
    let output_dir_arg = if args.len() >= 3 { &args[2] } else { "function_selectors" };
    let output_dir = Path::new(output_dir_arg);

    // Compile contracts using 'forge compile' <-- this is required to generate the ABI
    // WARNING: this script assumes that 'forge' is installed in the current project.
    println!("Compiling contracts with 'forge compile'...");
    let compile_output = Command::new("forge")
        .arg("compile")
        .output()
        .expect("Error running 'forge compile'");

    if !compile_output.status.success() {
        eprintln!(
            "Compilation failed: {}",
            String::from_utf8_lossy(&compile_output.stderr)
        );
        std::process::exit(1);
    }
    println!("Contracts successfully compiled.");

    // Create sub-folders: selectors & events
    let selectors_dir = output_dir.join("selectors");
    let events_dir = output_dir.join("events");
    create_dir_all(&selectors_dir).expect("'selectors' folder couldn't be created");
    create_dir_all(&events_dir).expect("'events' folder couldn't be created");

    // Definitions for csv rows
    // Array for each row: [contractName, <signature>, <selector or topic>]
    let mut csv_events: Vec<[String; 3]> = Vec::new();
    let mut csv_selectors: Vec<[String; 3]> = Vec::new();
    // Adding headers
    csv_events.push(["contractName".to_string(), "event".to_string(), "topic".to_string()]);
    csv_selectors.push(["contractName".to_string(), "function".to_string(), "selector".to_string()]);

    // Recursively check the contract's folder looking for .sol files
    for entry in WalkDir::new(contracts_path)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| {
            e.path().extension().map(|ext| ext == "sol").unwrap_or(false)
        })
    {
        let file_path = entry.path().to_str().unwrap();
        // Assuming that the contract's name matches the file's name.
        let contract_name = entry
            .path()
            .file_stem()
            .and_then(|s| s.to_str())
            .expect("Contract's name couldn't be extracted");

        println!("Checking contract '{}' from file '{}'", contract_name, file_path);

        // Running: 'forge inspect <contract_file.sol>:<ContractName> abi'
        let output = Command::new("forge")
            .args(&["inspect", &format!("{}:{}", file_path, contract_name), "abi"])
            .output()
            .expect("Error running 'forge inspect'");

        if !output.status.success() {
            eprintln!(
                "'forge inspect' command failed for {}: {}",
                file_path,
                String::from_utf8_lossy(&output.stderr)
            );
            continue;
        }

        let abi_json = String::from_utf8_lossy(&output.stdout);
        let abi_entries: Vec<AbiEntry> = serde_json::from_str(&abi_json)
            .expect("Error parsing ABI's JSON");

        // Definitions for contract's data (for CSV files & individual txt files)
        let mut contract_events: Vec<(String, String)> = Vec::new();
        let mut contract_functions: Vec<(String, String)> = Vec::new();

        // Parsing each ABI's entry
        for entry in abi_entries {
            match entry.kind.as_str() {
                "function" => {
                    let name = entry.name.unwrap_or_else(|| "unknown".to_string());
                    let input_types = entry
                        .inputs
                        .unwrap_or_default()
                        .into_iter()
                        .map(|inp| inp.param_type)
                        .collect::<Vec<_>>()
                        .join(",");
                    let signature = format!("{}({})", name, input_types);
                    
                    // Selector: take keccak256 hash & extract 4 1st bytes
                    let hash = Keccak256::digest(signature.as_bytes());
                    let selector = &hash[..4];
                    let selector_hex = format!("0x{}", hex::encode(selector));
                    
                    contract_functions.push((signature, selector_hex));
                },
                "event" => {
                    let name = entry.name.unwrap_or_else(|| "unknown".to_string());
                    let input_types = entry
                        .inputs
                        .unwrap_or_default()
                        .into_iter()
                        .map(|inp| inp.param_type)
                        .collect::<Vec<_>>()
                        .join(",");
                    let signature = format!("{}({})", name, input_types);
                    
                    // Topic: take full keccak256 hash
                    let hash = Keccak256::digest(signature.as_bytes());
                    let topic_hex = format!("0x{}", hex::encode(hash));
                    
                    if entry.anonymous {
                        contract_events.push((format!("{} [anonymous]", signature), topic_hex));
                    } else {
                        contract_events.push((signature, topic_hex));
                    }
                },
                _ => {} // Other types ignored
            }
        }

        // Write individual .txt files (optional)
        // let selectors_output_file = selectors_dir.join(format!("{}.txt", contract_name));
        // let events_output_file = events_dir.join(format!("{}.txt", contract_name));
        // {
        //     let mut file_func = File::create(&selectors_output_file)
        //         .unwrap_or_else(|_| panic!("file {:?} couldn't be created", selectors_output_file));
        //     for (sig, selector_hex) in &contract_functions {
        //         writeln!(file_func, "{} -> {}", sig, selector_hex)
        //             .unwrap_or_else(|_| panic!("Error writting in {:?}", selectors_output_file));
        //     }
        // }
        // {
        //     let mut file_event = File::create(&events_output_file)
        //         .unwrap_or_else(|_| panic!("File {:?} couldn't be created", events_output_file));
        //     for (sig, topic_hex) in &contract_events {
        //         writeln!(file_event, "{} -> {}", sig, topic_hex)
        //             .unwrap_or_else(|_| panic!("Error writting in {:?}", events_output_file));
        //     }
        // }
        // println!("txt files successfully written for '{}'", contract_name);

        // Add rows to the global CSV for each contract.
        // 1st row: contract's name
        csv_events.push([contract_name.to_string(), "".to_string(), "".to_string()]);
        csv_selectors.push([contract_name.to_string(), "".to_string(), "".to_string()]);
        // Then, a row for each event (leaving 1st column empty)
        for (sig, topic_hex) in contract_events {
            csv_events.push(["".to_string(), sig, topic_hex]);
        }
        // Then, a row for each function
        for (sig, selector_hex) in contract_functions {
            csv_selectors.push(["".to_string(), sig, selector_hex]);
        }
    }

    // Finnaly, we write CSV files in their respective sub-folders
    let events_csv_path = events_dir.join("events.csv");
    let selectors_csv_path = selectors_dir.join("selectors.csv");

    {
        let file = File::create(&events_csv_path).expect("events' CSV file couldn't be created");
        let mut writer = BufWriter::new(file);
        for row in csv_events {
            let line = format!(
                "{},{},{}\n",
                escape_csv_field(&row[0]),
                escape_csv_field(&row[1]),
                escape_csv_field(&row[2])
            );
            writer.write_all(line.as_bytes()).expect("Error writting on event's CSV file");
        }
    }
    {
        let file = File::create(&selectors_csv_path).expect("selectors' CSV file couldn't be created");
        let mut writer = BufWriter::new(file);
        for row in csv_selectors {
            let line = format!(
                "{},{},{}\n",
                escape_csv_field(&row[0]),
                escape_csv_field(&row[1]),
                escape_csv_field(&row[2])
            );
            writer.write_all(line.as_bytes()).expect("Error writting on selector's CSV file");
        }
    }
    println!("CSV files generated:\n  Events -> {:?}\n  Selectors -> {:?}", events_csv_path, selectors_csv_path);
}