static USAGE: &str = r#"
Convert JSON to CSV.

The JSON data is expected to be non-empty and non-nested as either:

1. An array of objects where:
   A. All objects are non-empty and have the same keys.
   B. Values are not objects or arrays.
2. An object where values are not objects or arrays.

If your JSON data is not in the expected format and/or is nested or complex, try using
the --jaq option to pass a jq-like filter before parsing with the above constraints.

As an example, say we have the following JSON data in a file fruits.json:

[
    {
        "fruit": "apple",
        "price": 2.50
    },
    {
        "fruit": "banana",
        "price": 3.00
    }
]

To convert it to CSV format run:

qsv json fruits.json

And the following is printed to the terminal:

fruit,price
apple,2.5
banana,3.0

Note: Trailing zeroes in decimal numbers after the decimal are truncated (2.50 becomes 2.5).

If the JSON data was provided using stdin then either use - or do not provide a file path.
For example you may copy the JSON data above to your clipboard then run:

qsv clipboard | qsv json

When JSON data is nested or complex, try using the --jaq option and provide a filter value.
The --jaq option uses jaq (like jq). You may learn more here: https://github.com/01mf02/jaq

For example we have a .json file with a "data" key and the value being the same array as before:

{
    "data": [...]
}

We may run the following to select the JSON file and convert the nested array to CSV:

qsv prompt -F json | qsv json --jaq .data

For more examples, see https://github.com/jqnatividad/qsv/blob/master/tests/test_json.rs.

Usage:
    qsv json [options] [<input>]
    qsv json --help

json options:
    --jaq <filter>         Filter JSON data using jaq syntax (https://github.com/01mf02/jaq).

Common options:
    -h, --help             Display this message
    -o, --output <file>    Write output to <file> instead of stdout.
"#;

use std::{env, io::Read};

use jaq_interpret::{Ctx, FilterT, ParseCtx, RcIter, Val};
use json_objects_to_csv::{flatten_json_object::Flattener, Json2Csv};
use serde::Deserialize;

use crate::{config, select::SelectColumns, util, CliError, CliResult};

#[derive(Deserialize)]
struct Args {
    arg_input:   Option<String>,
    flag_jaq:    Option<String>,
    flag_output: Option<String>,
}

impl From<json_objects_to_csv::Error> for CliError {
    fn from(err: json_objects_to_csv::Error) -> Self {
        match err {
            json_objects_to_csv::Error::Flattening(err) => {
                CliError::Other(format!("Flattening error: {err}"))
            },
            json_objects_to_csv::Error::FlattenedKeysCollision => {
                CliError::Other(format!("Flattening Key Collision error: {err}"))
            },
            json_objects_to_csv::Error::WrittingCSV(err) => {
                CliError::Other(format!("Writing CSV error: {err}"))
            },
            json_objects_to_csv::Error::ParsingJson(err) => {
                CliError::Other(format!("Parsing JSON error: {err}"))
            },
            json_objects_to_csv::Error::InputOutput(err) => CliError::Io(err),
            json_objects_to_csv::Error::IntoFile(err) => CliError::Io(err.into()),
        }
    }
}

pub fn run(argv: &[&str]) -> CliResult<()> {
    fn get_value_from_stdin() -> CliResult<serde_json::Value> {
        // Create a buffer in memory for stdin
        let mut buffer: Vec<u8> = Vec::new();
        let stdin = std::io::stdin();
        let mut stdin_handle = stdin.lock();
        stdin_handle.read_to_end(&mut buffer)?;
        drop(stdin_handle);

        // Return the JSON contents of the buffer as serde_json::Value
        match serde_json::from_slice(&buffer) {
            Ok(value) => Ok(value),
            Err(err) => fail_clierror!("Failed to parse JSON from stdin: {err}"),
        }
    }

    fn get_value_from_path(path: String) -> CliResult<serde_json::Value> {
        // Open the file in read-only mode with buffer.
        let file = std::fs::File::open(path)?;
        let reader = std::io::BufReader::new(file);

        // Return the JSON contents of the file as serde_json::Value
        match serde_json::from_reader(reader) {
            Ok(value) => Ok(value),
            Err(err) => fail_clierror!("Failed to parse JSON from file: {err}"),
        }
    }

    let args: Args = util::get_args(USAGE, argv)?;

    let flattener = Flattener::new();
    let mut value = if let Some(path) = args.arg_input {
        get_value_from_path(path)?
    } else {
        get_value_from_stdin()?
    };

    if value.is_null() {
        return fail_clierror!("No JSON data found.");
    }

    if let Some(filter) = args.flag_jaq {
        // Parse jaq filter based on JSON input
        let mut defs = ParseCtx::new(Vec::new());
        let (f, _errs) = jaq_parse::parse(filter.as_str(), jaq_parse::main());
        let f = defs.compile(f.unwrap());
        let inputs = RcIter::new(core::iter::empty());
        let out = f
            .run((Ctx::new([], &inputs), Val::from(value.clone())))
            .filter_map(std::result::Result::ok);

        #[allow(clippy::from_iter_instead_of_collect)]
        let jaq_value = serde_json::Value::from_iter(out);

        // from_iter creates a Value::Array even if the JSON data is an array,
        // so we unwrap this generated Value::Array to get the actual filtered output.
        // This allows the user to filter with '.data' for {"data": [...]} instead of not being able
        // to use '.data'. Both '.data' and '.data[]' should work with this implementation.
        value = if jaq_value
            .as_array()
            .is_some_and(|arr| arr.first().is_some_and(serde_json::Value::is_array))
        {
            jaq_value.as_array().unwrap().first().unwrap().to_owned()
        } else {
            jaq_value
        };
    }

    if value.is_null() {
        return fail_clierror!("No JSON data found.");
    }

    let first_dict = if value.is_array() {
        value
            .as_array()
            .and_then(|arr| arr.first())
            .and_then(|val| val.as_object())
            .ok_or_else(|| CliError::Other("Expected an array of objects in JSON".to_string()))?
    } else {
        value
            .as_object()
            .ok_or_else(|| CliError::Other("Expected a JSON object".to_string()))?
    };
    if first_dict.is_empty() {
        return Err(CliError::Other(
            "Expected a non-empty JSON object".to_string(),
        ));
    }
    let mut headers: Vec<&str> = Vec::new();
    for key in first_dict.keys() {
        headers.push(key.as_str());
    }

    let empty_values = vec![serde_json::Value::Null; 1];
    let values = if value.is_array() {
        value.as_array().unwrap_or(&empty_values)
    } else {
        &vec![value.clone()]
    };

    // STEP 1: create an intermediate CSV tempfile from the JSON data
    // we need to do this so we can use qsv select to reorder headers to first dict's keys order
    // as the order of the headers in the CSV file is not guaranteed to be the same as the order of
    // the keys in the JSON object
    let temp_dir = env::temp_dir();
    let intermediate_csv = temp_dir.join("intermediate.csv");

    // this is in a block so that the intermediate_csv_writer is automatically flushed
    // w/o triggering the borrow checker for the intermediate_csv variable when it goes out of scope
    {
        let intermediate_csv_file = std::io::BufWriter::with_capacity(
            config::DEFAULT_WTR_BUFFER_CAPACITY,
            std::fs::File::create(&intermediate_csv)?,
        );
        let intermediate_csv_writer = csv::WriterBuilder::new().from_writer(intermediate_csv_file);
        Json2Csv::new(flattener).convert_from_array(values, intermediate_csv_writer)?;
    }

    // STEP 2: select the columns in the order of the first dict's keys
    let sel_cols = SelectColumns::parse(&headers.join(","))?;

    let sel_rconfig = config::Config::new(&Some(intermediate_csv.to_string_lossy().into_owned()));
    let mut intermediate_csv_rdr = sel_rconfig.reader()?;
    let byteheaders = intermediate_csv_rdr.byte_headers()?.clone();

    // and write the selected columns to the final CSV file
    let sel = sel_rconfig.select(sel_cols).selection(&byteheaders)?;
    let mut record = csv::ByteRecord::new();
    let mut final_csv_wtr = config::Config::new(&args.flag_output).writer()?;
    final_csv_wtr.write_record(sel.iter().map(|&i| &byteheaders[i]))?;
    while intermediate_csv_rdr.read_byte_record(&mut record)? {
        final_csv_wtr.write_record(sel.iter().map(|&i| &record[i]))?;
    }

    Ok(final_csv_wtr.flush()?)
}
