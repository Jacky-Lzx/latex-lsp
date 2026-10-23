use std::path::Path;

/// Construct a normal-mode control file from the installed Biber defaults.
/// Use normal-mode set processing without requiring biblatex.sty.
pub(crate) fn control_file() -> String {
    let output = std::process::Command::new("biber")
        .arg("--tool-config")
        .output()
        .unwrap();
    assert!(output.status.success());
    let config_path = String::from_utf8(output.stdout).unwrap();
    let config_path = Path::new(config_path.trim());
    let config = std::fs::read_to_string(config_path).unwrap();
    let constants =
        std::fs::read_to_string(config_path.parent().unwrap().join("Constants.pm")).unwrap();
    let version = constants
        .split("$BCF_VERSION = '")
        .nth(1)
        .unwrap()
        .split('\'')
        .next()
        .unwrap();
    let mut body = config
        .split_once("<config>")
        .unwrap()
        .1
        .split_once("</config>")
        .unwrap()
        .0
        .to_string();
    while let Some(start) = body.find("<sourcemap>") {
        let end = start + body[start..].find("</sourcemap>").unwrap() + "</sourcemap>".len();
        body.replace_range(start..end, "");
    }
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<controlfile xmlns="https://sourceforge.net/projects/biblatex" version="{version}" bltxversion="native-test">
{body}
<options component="biblatex" type="global"><option type="singlevalued"><key>sortingtemplatename</key><value>tool</value></option><option type="singlevalued"><key>sortlocale</key><value>en-US</value></option></options>
<bibdata section="0"><datasource type="file" datatype="bibtex" glob="false">refs.data</datasource></bibdata>
<section number="0"><citekey order="1" intorder="1">*</citekey></section></controlfile>"#
    )
}
