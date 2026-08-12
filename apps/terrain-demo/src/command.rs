use std::{
    ffi::{OsStr, OsString},
    io,
    path::{Path, PathBuf},
};

use crate::AppResult;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct RunCommand {
    pub(crate) source: PathBuf,
    pub(crate) index: PathBuf,
    pub(crate) workspace: PathBuf,
    pub(crate) landxml: PathBuf,
    pub(crate) document_date: String,
    pub(crate) document_time: String,
    pub(crate) qa_sample: bool,
    pub(crate) assert_crs_metric: bool,
    pub(crate) correction_revert_ordinal: Option<u64>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum Command {
    Help,
    Run(RunCommand),
}

impl Command {
    pub(crate) fn parse(arguments: impl IntoIterator<Item = OsString>) -> AppResult<Self> {
        let mut arguments = arguments.into_iter();
        let mut paths = Vec::new();
        let mut document_date = None;
        let mut document_time = None;
        let mut qa_sample = false;
        let mut assert_crs_metric = false;
        let mut correction_revert_ordinal = None;
        let mut positional_only = false;

        while let Some(argument) = arguments.next() {
            if !positional_only
                && (argument == OsStr::new("--help") || argument == OsStr::new("-h"))
            {
                return Ok(Self::Help);
            }
            if !positional_only && argument == OsStr::new("--") {
                positional_only = true;
            } else if !positional_only && argument == OsStr::new("--qa-sample") {
                require_new_flag(qa_sample, "--qa-sample")?;
                qa_sample = true;
            } else if !positional_only && argument == OsStr::new("--assert-crs-metric") {
                require_new_flag(assert_crs_metric, "--assert-crs-metric")?;
                assert_crs_metric = true;
            } else if !positional_only && argument == OsStr::new("--exercise-correction-revert") {
                let value = required_option_value(&mut arguments, "--exercise-correction-revert")?;
                let ordinal = value.parse::<u64>().map_err(|_| {
                    invalid_input(
                        "--exercise-correction-revert requires a non-negative Point ordinal",
                    )
                })?;
                if correction_revert_ordinal.replace(ordinal).is_some() {
                    return Err(invalid_input(
                        "--exercise-correction-revert was supplied more than once",
                    )
                    .into());
                }
            } else if !positional_only && argument == OsStr::new("--date") {
                let value = required_option_value(&mut arguments, "--date")?;
                set_once(&mut document_date, value, "--date")?;
            } else if !positional_only && argument == OsStr::new("--time") {
                let value = required_option_value(&mut arguments, "--time")?;
                set_once(&mut document_time, value, "--time")?;
            } else if !positional_only && argument.to_string_lossy().starts_with('-') {
                return Err(invalid_input(format!(
                    "unknown option {}; use --help for usage",
                    Path::new(&argument).display()
                ))
                .into());
            } else {
                paths.push(PathBuf::from(argument));
            }
        }

        let [source, index, workspace, landxml]: [PathBuf; 4] =
            paths.try_into().map_err(|paths: Vec<PathBuf>| {
                invalid_input(format!(
                    "expected SOURCE, INDEX, WORKSPACE, and LANDXML paths; received {}",
                    paths.len()
                ))
            })?;
        Ok(Self::Run(RunCommand {
            source,
            index,
            workspace,
            landxml,
            document_date: document_date
                .ok_or_else(|| invalid_input("missing required --date YYYY-MM-DD"))?,
            document_time: document_time
                .ok_or_else(|| invalid_input("missing required --time HH:MM:SSZ"))?,
            qa_sample,
            assert_crs_metric,
            correction_revert_ordinal,
        }))
    }
}

pub(crate) fn print_usage() {
    println!(
        "{}",
        concat!(
            "Usage: terrain-demo [OPTIONS] SOURCE INDEX WORKSPACE LANDXML\n",
            "\n",
            "Required deterministic document options:\n",
            "  --date YYYY-MM-DD       LandXML document date; no clock is read\n",
            "  --time HH:MM:SSZ        LandXML UTC document time; no clock is read\n",
            "\n",
            "Optional:\n",
            "  --qa-sample             Evaluate one Surface vertex and one deliberate gap\n",
            "  --assert-crs-metric     Assert Source X/Y/Z are metric metres\n",
            "  --exercise-correction-revert ORDINAL\n",
            "                           Set one exact Ground Point to class 1, derive, Revert, and verify restoration\n",
            "  -h, --help              Show this help\n",
            "\n",
            "The LAS classification Attribute (ID 6) and Ground class (2) are fixed.\n",
            "INDEX is built or opened; WORKSPACE is created or opened; LANDXML is never replaced.",
        )
    );
}

fn required_option_value(
    arguments: &mut impl Iterator<Item = OsString>,
    option: &'static str,
) -> Result<String, io::Error> {
    let value = arguments
        .next()
        .ok_or_else(|| invalid_input(format!("{option} requires a value")))?;
    value.into_string().map_err(|_| {
        invalid_input(format!(
            "{option} requires Unicode text; paths may remain non-Unicode"
        ))
    })
}

fn set_once(slot: &mut Option<String>, value: String, option: &'static str) -> AppResult<()> {
    if slot.replace(value).is_some() {
        return Err(invalid_input(format!("{option} was supplied more than once")).into());
    }
    Ok(())
}

fn require_new_flag(already_set: bool, option: &'static str) -> Result<(), io::Error> {
    if already_set {
        Err(invalid_input(format!(
            "{option} was supplied more than once"
        )))
    } else {
        Ok(())
    }
}

fn invalid_input(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, message.into())
}
