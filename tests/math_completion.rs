use lsp_types::Url;
use texquill::{language::features, project::workspace::Workspace, text::Text};

fn labels(marked: &str) -> Vec<String> {
    let offset = marked.find('¦').unwrap();
    let source = marked.replace('¦', "");
    let uri = Url::parse("file:///math-completion/main.tex").unwrap();
    let mut workspace = Workspace::default();
    workspace.open(uri.clone(), source.clone(), 1);
    features::completion(&workspace, &uri, Text::new(source).position(offset))
        .into_iter()
        .map(|item| item.label)
        .collect()
}

#[test]
fn math_commands_require_math_mode() {
    for name in ["alpha", "sum", "frac", "sin", "widehat", "mathbb", "left"] {
        assert!(
            !labels(&format!("Text \\{name}¦")).contains(&name.into()),
            "text mode offered {name}"
        );
        assert!(labels(&format!("$\\{name}¦$")).contains(&name.into()));
    }
}

#[test]
fn formulas_and_text_regions_follow_delimiters_and_environment_boundaries() {
    for marked in [
        r"$\al¦$",
        r"$$\al¦$$",
        r"\(\al¦\)",
        r"\[\al¦\]",
        r"\ensuremath{\al¦}",
        r"\ensuremath{\al¦",
        r"$\text{words $\al¦$}$",
        r"$\text{words}\al¦$",
        r"$\text{\ensuremath{\al¦}}$",
        r"$\frac{\al¦}{b}$",
        "$a% $ ignored\n\\al¦$",
        r"$\verb|$|\al¦$",
        r"$\detokenize{$}\al¦$",
        r"$\meaning$\al¦$",
        r"\begin{align}a & = b \\\al¦\end{align}",
    ] {
        assert!(labels(marked).contains(&"alpha".into()), "{marked}");
    }
    for env in [
        "math",
        "displaymath",
        "equation",
        "equation*",
        "eqnarray",
        "align",
        "align*",
        "alignat",
        "flalign",
        "gather",
        "multline*",
    ] {
        let inside = format!("\\begin{{{env}}}\n\\al¦\n\\end{{{env}}}");
        assert!(labels(&inside).contains(&"alpha".into()), "{inside}");
        let after = format!("\\begin{{{env}}}x\\end{{{env}}} \\al¦");
        assert!(!labels(&after).contains(&"alpha".into()), "{after}");
    }
    for marked in [
        r"中文😀 \al¦",
        r"$x$ \al¦",
        r"$$x$$ \al¦",
        r"\(x\) \al¦",
        r"\[x\] \al¦",
        r"\$\al¦",
        r"\\(\al¦",
        r"\\[\al¦",
        "% $ \n ignored\n\\al¦",
        r"\verb|$|\al¦",
        r"\detokenize{$}\al¦",
        r"\string$\al¦",
        r"\meaning$\al¦",
        r"\let\saved=$\al¦",
        r"\begin{verbatim}$\end{verbatim}\al¦",
        r"\ensuremath{x}\al¦",
        r"$\text{\al¦}$",
        r"$\text{nested {text}\al¦}$",
        r"$\mbox{\al¦}$",
        r"$\textrm{\al¦}$",
        r"$\operatorname*{\al¦}$",
        r"$\text{\ensuremath{x}\al¦}$",
        r"\newcommand{\formula}{$x$}\al¦",
        r"% \al¦",
        r"\verb|\al¦|",
    ] {
        assert!(!labels(marked).contains(&"alpha".into()), "{marked}");
    }
}

#[test]
fn package_math_commands_are_filtered_and_general_commands_are_preserved() {
    let packages = r"\usepackage{mathtools,amssymb,booktabs}";
    for name in [
        "dfrac",
        "binom",
        "coloneqq",
        "therefore",
        "mathbb",
        "mathfrak",
    ] {
        let text = format!("{packages} \\{name}¦");
        assert!(!labels(&text).contains(&name.into()), "{text}");
        let math = format!("{packages} $\\{name}¦$");
        assert!(labels(&math).contains(&name.into()), "{math}");
    }
    for name in [
        "section",
        "textbf",
        "underline",
        "DeclareMathOperator",
        "toprule",
    ] {
        let text = format!("{packages} \\{name}¦");
        assert!(labels(&text).contains(&name.into()), "{text}");
    }
    for marked in [
        r"\meaning\al¦",
        r"\let\saved=\al¦",
        r"\newcommand{\al¦}",
        r"\newcommand{\alpha}{text}\al¦",
    ] {
        assert!(labels(marked).contains(&"alpha".into()), "{marked}");
    }
}
