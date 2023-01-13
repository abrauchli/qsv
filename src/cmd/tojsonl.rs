static USAGE: &str = r#"
Smartly converts CSV to a newline-delimited JSON (JSONL/NDJSON).

By scanning the CSV first, it "smartly" infers the appropriate JSON data type
for each column.

It will infer a column as boolean if it only has a domain of two values,
and the first character of the values are one of the following case-insensitive
combinations: t/f; t/null; 1/0; 1/null; y/n & y/null are treated as true/false.

For examples, see https://github.com/jqnatividad/qsv/blob/master/tests/test_tojsonl.rs.

Usage:
    qsv tojsonl [options] [<input>]
    qsv tojsonl --help

Tojsonl optionns:
    -j, --jobs <arg>       The number of jobs to run in parallel.
                           When not set, the number of jobs is set to the
                           number of CPUs detected.

Common options:
    -h, --help             Display this message
    -d, --delimiter <arg>  The field delimiter for reading CSV data.
                           Must be a single character. (default: ,)
    -o, --output <file>    Write output to <file> instead of stdout.
"#;

use std::{env::temp_dir, fs::File, path::Path, str::FromStr};

use serde::Deserialize;
use serde_json::{Map, Value};
use strum_macros::EnumString;
use uuid::Uuid;

use super::schema::infer_schema_from_stats;
use crate::{
    config::{Config, Delimiter},
    util, CliError, CliResult,
};

#[derive(Deserialize, Clone)]
struct Args {
    arg_input:      Option<String>,
    flag_jobs:      Option<usize>,
    flag_delimiter: Option<Delimiter>,
    flag_output:    Option<String>,
}

impl From<std::fmt::Error> for CliError {
    fn from(err: std::fmt::Error) -> CliError {
        CliError::Other(err.to_string())
    }
}

#[derive(PartialEq, EnumString)]
#[strum(ascii_case_insensitive)]
enum JsonlType {
    Boolean,
    String,
    Number,
    Integer,
    Null,
}

pub fn run(argv: &[&str]) -> CliResult<()> {
    let preargs: Args = util::get_args(USAGE, argv)?;
    let mut args = preargs.clone();
    let conf = Config::new(&args.arg_input).delimiter(args.flag_delimiter);
    let mut is_stdin = false;

    let stdin_fpath = format!("{}/{}.csv", temp_dir().to_string_lossy(), Uuid::new_v4());
    let stdin_temp = stdin_fpath.clone();

    // if using stdin, we create a stdin.csv file as stdin is not seekable and we need to
    // open the file multiple times to compile stats/unique values, etc.
    let input_filename = if preargs.arg_input.is_none() {
        let mut stdin_file = File::create(stdin_fpath.clone())?;
        let stdin = std::io::stdin();
        let mut stdin_handle = stdin.lock();
        std::io::copy(&mut stdin_handle, &mut stdin_file)?;
        args.arg_input = Some(stdin_fpath.clone());
        is_stdin = true;
        stdin_fpath
    } else {
        let filename = Path::new(args.arg_input.as_ref().unwrap())
            .file_name()
            .unwrap()
            .to_string_lossy()
            .to_string();
        filename
    };
    // we're calling the schema command to infer data types and enums
    let schema_args = crate::cmd::schema::Args {
        // we only do three, as we're only inferring boolean based on enum
        flag_enum_threshold:  3,
        flag_strict_dates:    false,
        flag_pattern_columns: crate::select::SelectColumns::parse("")?,
        // json doesn't have a date type, so don't infer dates
        flag_dates_whitelist: "none".to_string(),
        flag_prefer_dmy:      false,
        flag_stdout:          false,
        flag_jobs:            Some(util::njobs(args.flag_jobs)),
        flag_no_headers:      false,
        flag_delimiter:       args.flag_delimiter,
        arg_input:            args.arg_input.clone(),
    };
    // build schema for each field by their inferred type, min/max value/length, and unique values
    let properties_map: Map<String, Value> =
        match infer_schema_from_stats(&schema_args, &input_filename) {
            Ok(map) => map,
            Err(e) => {
                return fail_clierror!("Failed to infer field types via stats and frequency: {e}");
            }
        };

    let mut rdr = if is_stdin {
        Config::new(&Some(stdin_temp))
            .delimiter(args.flag_delimiter)
            .reader()?
    } else {
        conf.reader()?
    };
    let mut wtr = Config::new(&args.flag_output)
        .flexible(true)
        .no_headers(true)
        .quote_style(csv::QuoteStyle::Never)
        .writer()?;

    let headers = rdr.headers()?.clone();

    // create a vec lookup about inferred field data types
    let mut field_type_vec: Vec<JsonlType> = Vec::with_capacity(headers.len());
    for (_field_name, field_def) in properties_map.iter() {
        let Some(field_map) = field_def.as_object() else { return fail!("Cannot create field map") };
        let prelim_type = field_map.get("type").unwrap();
        let field_values_enum = field_map.get("enum");

        // log::debug!("prelim_type: {prelim_type} field_values_enum: {field_values_enum:?}");

        // check if a field has a boolean data type
        // by checking its enum constraint
        if let Some(domain) = field_values_enum {
            if let Some(vals) = domain.as_array() {
                // if this field only has a domain of two values
                if vals.len() == 2 {
                    let val1 = if vals[0].is_null() {
                        '_'
                    } else {
                        // if its a string
                        // get the first character of val1 lowercase
                        if let Some(str_val) = vals[0].as_str() {
                            first_lower_char(str_val)
                        } else if let Some(int_val) = vals[0].as_u64() {
                            // its an integer (as we only do enum constraints
                            // for string and integers)
                            match int_val {
                                1 => '1',
                                0 => '0',
                                _ => '*', // its something else
                            }
                        } else {
                            '*'
                        }
                    };
                    // same as above, but for the 2nd value
                    let val2 = if vals[1].is_null() {
                        '_'
                    } else if let Some(str_val) = vals[1].as_str() {
                        first_lower_char(str_val)
                    } else if let Some(int_val) = vals[1].as_u64() {
                        match int_val {
                            1 => '1',
                            0 => '0',
                            _ => '*',
                        }
                    } else {
                        '*'
                    };
                    // log::debug!("val1: {val1} val2: {val2}");

                    // check if the domain of two values is truthy or falsy
                    // i.e. starts with case-insensitive "t", "1", "y" are truthy values
                    // ot "f", "0", "n" or null are falsy values
                    // if it is, infer a boolean field
                    if let ('t', 'f' | '_')
                    | ('f' | '_', 't')
                    | ('1', '0' | '_')
                    | ('0' | '_', '1')
                    | ('y', 'n' | '_')
                    | ('n' | '_', 'y') = (val1, val2)
                    {
                        field_type_vec.push(JsonlType::Boolean);
                        continue;
                    }
                }
            }
        }

        let temp_str = prelim_type.as_array().unwrap()[0]
            .as_str()
            .unwrap_or_default();
        field_type_vec.push(JsonlType::from_str(temp_str).unwrap_or(JsonlType::String));
    }

    // amortize allocs
    let mut record = csv::StringRecord::new();
    #[allow(unused_assignments)]
    let mut temp_str = String::with_capacity(100);
    #[allow(unused_assignments)]
    let mut temp_str2 = String::with_capacity(50);

    // write jsonl file
    while rdr.read_record(&mut record)? {
        use std::fmt::Write as _;

        temp_str.clear();
        record.trim();
        write!(temp_str, "{{")?;
        for (idx, field) in record.iter().enumerate() {
            let field_val = if let Some(field_type) = field_type_vec.get(idx) {
                match field_type {
                    JsonlType::Integer | JsonlType::Number => field,
                    JsonlType::Boolean => {
                        if let 't' | 'y' | '1' = first_lower_char(field) {
                            "true"
                        } else {
                            "false"
                        }
                    }
                    JsonlType::Null => "null",
                    JsonlType::String => {
                        if field.is_empty() {
                            "null"
                        } else {
                            temp_str2 = format!(r#""{}""#, field.escape_default());
                            &temp_str2
                        }
                    }
                }
            } else {
                "null"
            };
            if field_val.is_empty() {
                write!(temp_str, r#""{}":null,"#, &headers[idx])?;
            } else {
                write!(temp_str, r#""{}":{field_val},"#, &headers[idx])?;
            }
        }
        temp_str.pop(); // remove last comma
        temp_str.push('}');
        record.clear();
        record.push_field(&temp_str);
        wtr.write_record(&record)?;
    }

    Ok(wtr.flush()?)
}

#[inline]
fn first_lower_char(field_str: &str) -> char {
    field_str
        .trim_start()
        .chars()
        .next()
        .unwrap_or('_')
        .to_ascii_lowercase()
}
