# biblatex citations

[Documentation](README.md) · [Document Resource Loading](biblatex-resources.md) · [Document Key and Rename](bibliography.md) · [Build diagnostics](diagnostics.md)

Rerun requests for documentation tools and LaTeX will also go into [Compilation Result Analysis](build-analysis.md#document-processing-and-recompilation).

See [Bibliography entry sets](bibliography-sets.md) for independent key editing of the dynamic collection declaration `\defbibentryset{set key}{members}`.

Literature keys share completion, hover, navigation, citation lookup, highlighting, and cross-file renaming; fields, lists, and item types provide their own name completion and hover. The following commands also provide command snippets and signature help. Common missing literature warnings for quoting commands are located at the key name and support spelling repair; description text, tags with the same name, body text, comments, and verbatim text will be retained.

- [Fields, lists and name references](#fields-lists-and-name-references)
- [Item Data Context](#entry-data-context)
- [Field and List Access](#field-and-list-access)
- [Field and list conditions](#field-and-list-conditions)
- [Entry condition check](#entry-condition-check)
- [Single citation](#single-reference)
- [Multiple sets of references](#multiple-sets-of-references)
- [Roll number reference](#volume-number-reference)

## Fields, lists and name references

`citefield`, `citelist`, and `citename` print the fields, literal list, and name list of document entries respectively. They accept up to two prefixes, followed by a list of citation keys, an optional format name, and a required field or list name:

```tex
\citefield[see][12]{alpha,beta}[citefield]{title}
\citelist{alpha}{location}
\citename{alpha}[citename]{author}
```

The citation key list supports completion, hover, navigation, reference search, highlighting and cross-file renaming, including cross-line and unclosed lists; document renaming retains the same text in the format name, field name, list name and description. The command snippet only inserts two required parameters, and signature help track descriptions, keys, formats, and fields respectively, and also retain user-defined command signatures.

The last parameter also provides field or list name completion and hover instructions: `citefield` recommends `title`, `year`, `doi` and other output fields, `citelist` recommends `publisher`, `location` Literal list such as `citename` and `author`, `editor`, `translator` and other name lists are recommended. Candidates use common output names in the default data model. Names that are only for input processing such as `date` and `sortname` are not listed, and custom models are not verified.

Completion filters by ASCII ignoring case and inserts standard lowercase names, replacing the entire current name, retaining descriptions, formatting, comments and calling foreign text; supports cross-line and incomplete parameters. These names are independent of the citation key, and even if there are entries with the same name in the database, document jumps or renames will not be triggered. Dynamic names, redundant descriptions, verbatim text, and commands that have been redefined by the user do not provide built-in selector hints.

The compiler may not report a missing citation key until after it has read the format and fields, or even after reading the next command. The diagnostics will check the complete call range and locate warnings and spelling fixes to the actual key names; the result analysis will continue with key deduplication. None of the three commands interpret asterisks or capitalization, and do not interpret redundant descriptions or dynamic keys as static citation keys.

## Entry data context

`\entrydata{key}{code}` and `\entrydata*{key}{code}` execute code in the data context of the specified entry. The first parameter is processed as a single static citation key and supports completion, definition jump, hover, reference search, highlighting, semantic token, selection range and cross-file renaming; the ordinary text in the second parameter is retained, and the nested supported citation commands still participate in document editing independently.

```tex
\entrydata{alpha}{\printfield{title}}
\entrydata*{beta}{\printnames{author}}
```

The signature help distinguishes between `{key}` and `{code}`, and the command snippet provides two required parameters. Supports spaces before and after asterisks, comments, and cross-line key names. Completion retains comments; only command names are completed when there are parameters. Keys allow Unicode, quotes, and normal parentheses, and do not guess comma lists, semantic whitespace, nested grouping, or macro expansion as single keys. The data context's field expansion, reference tracking, or execution order is not inferred here, nor is the location of the ordinary `Citation … undefined` warning picked up.

## Field and list access

`printfield`, `printlist` and `printnames` provide command snippets and signature help: field output accepts optional formats, literal lists and name lists also accept the optional `[start-stop]` range. Optional parameters end at the first unprotected `]`, and the literal `[` does not increase nesting. The fragment inserts only the required field or list names, leaving existing parameters as they are; these names are not treated as citation keys.

The last parameter of the three print commands share the above fields, literal list and name list completion and hover instructions, and the format and range parameters remain independent.

Field reads for `\thefield{field}` and `\strfield{field}` also provide snippets, signature help, field completion and hover, and do not accept optional formats or asterisks.

## Field and list conditions

The first parameters of `\iffieldundef{field}{true}{false}`, `\iflistundef{literal list}{true}{false}` and `\ifnameundef{name list}{true}{false}` respectively suggest the names of the corresponding types. The ordinary text of the two branches remains the same; the signature help track the names and the two branches respectively. These commands do not accept optional formats or asterisks.

Field comparison also supports snippets and signature help: `\iffieldequalstr{field}{string}{true}{false}` compares fields and strings, `\iffieldequals{field}{command}{true}{false}` compares fields and command content, and `\iffieldsequal{field 1}{field 2}{true}{false}` compares two fields. The former two only provide field name completion in the first parameter, while the latter provides it in both field parameters; comparison strings and ordinary text in the two branches are not used as field names. `\iflistequals{literal list}{command}{true}{false}` and `\ifnameequals{name list}{command}{true}{false}` suggest literal lists and name lists respectively. These commands do not accept asterisks or optional formats, and do not perform comparisons or predict branch results.

The second parameters of the three `equals` commands are processed as macro name operands: completion only inserts the command name and does not append the calling fragment; the compared `\cite`, `\newcommand` or verbatim commands are not parsed as calls. The real commands in the branch remain independent. Like other command name operands, comparison commands that have been explicitly redefined in the current file are processed as ordinary calls; cross-file redefinition or group recovery will not be fully simulated.

User declarations and their derived commands will remove conflicting built-in selectors and signature help, such as `\thefield` generated by `\newcounter{field}` or conditional commands declared by `\newif\ifnameundef`; command name completion will no longer insert literature field placeholders, and will be restored after removing the override. Statements in other independent projects do not affect the current document.

## Entry condition check

`\ifentryexists{key}{true}{false}` and `\ifentryinbib{key}{true}{false}` respectively check whether the entry data exists and whether the entry is included in the literature list. The first parameter supports navigation, completion, hovering, reference search, highlighting, semantic token, selection range and renaming of a single static citation key; ordinary text in the two branches is retained, and nested references are independently indexed.

`\ifentrycategory{key}{category}{true}{false}` checks the category of the specified entry. The first parameter uses the same single-key editing rules; the second parameter belongs to an independent [category name](bibliography-categories.md) namespace, retaining ordinary text in the two code branches. Signature help indicate all four parameters individually.

The signature help distinguishes between keys, establishment codes and failure codes. The command snippet provides three required parameters. Keys use the same comment boundaries and static validation as `entrydata`, support Unicode, quotes and parentheses, comma lists and dynamic keys are not inferred. The two commands do not accept asterisks and optional descriptions, and do not receive ordinary missing literature reference warnings; the actual branch results and execution order are determined by biblatex.

`\ifentrytype{entry type}{true}{false}` Check the output type of the current entry. The first parameter provides completion and hovering of commonly used biblatex output types, such as `article`, `book`, `online`, `report` and `thesis`, the description is the same as `.bib` Entry types are shared; completion only replaces the name, retains comments, newlines and two branches, and also supports uncompleted parameters. There are three required parameters for fragment insertion, and the parameters prompt the tracking type and two branches. Names are not treated as citation keys or field names; asterisks, optional parameters, dynamic names, and user overrides do not provide built-in type hints.

Type candidates adopt common output names from the default data model and do not list input aliases such as `comment`, `string`, `preamble`, inheritance-only `xdata`, or `conference`, `phdthesis`; custom data models and entry type conversions are not inferred, and conditional results are not predicted.

## Single reference

| Command | Capitalized form | Purpose |
| --- | --- | --- |
| `cite` | `Cite` | General citation |
| `parencite` | `Parencite` | Bracket reference |
| `footcite` | `Footcite` | Footnote reference |
| `footcitetext` | `Footcitetext` | Footnote text without footnote mark |
| `smartcite` | `Smartcite` | Use text or footnote citations as appropriate |
| `supercite` | None | Superscript reference, requires a style that supports it |
| `textcite` | `Textcite` | Narrative Quote |
| `autocite` | `Autocite` | Select citation form according to configuration |
| `citeauthor` | `Citeauthor` | Author |
| `citeyear` | None | Year |
| `citetitle` | `Citetitle` | Title |
| `citedate` | None | Date |
| `citeurl` | None | URL |
| `notecite` | `Notecite` | Output description and record citation key |
| `pnotecite` | `Pnotecite` | Explanatory references in brackets |
| `fnotecite` | `Fnotecite` | Explanatory citations in footnotes |
| `fullcite` | None | Complete literature entry |
| `footfullcite` | None | Full entry in footnote |

The commands in the table recognize both normal and asterisk forms; spaces and comments are allowed before and after the asterisk. The typesetting meaning of the asterisk is determined by the command and style; when there is no independent asterisk implementation, biblatex uses the common implementation, such as `fullcite*`, `footfullcite*`. `citetitle*` uses the full title, `citedate*` contains date disambiguation information. Case recognition is based on the actual command, `Supercite`, `Citeurl` or `Citedate` are not inferred.

A single citation accepts up to two square bracket descriptions and a curly braced citation key list:

```tex
\Parencite[see][12]{alpha,beta}
\citetitle*{alpha}
\citeurl{alpha}
\notecite[see also][34]{beta}
\fullcite*{alpha}
```

Even if the description and key have the same name, only the key list will be edited. Completion supports spanning lines or unclosed lists, and only replaces the current key; command completion retains existing parameters. The signature help distinguishes pre-notes, post-notes and keys, and user-defined command signatures take precedence.

The `notecite` series does not print citation text, but still reads citation keys, participates in citation tracking, and reports missing keys, and therefore also participates in navigation and renaming. The specific behavior of `supercite` depends on the reference style; styles such as `numeric` ignore optional specifications and generate warnings, which are retained.

## Multiple sets of references

The following standard commands and asterisk forms are supported:

| Command | Capitalized form |
| --- | --- |
| `cites` | `Cites` |
| `parencites` | `Parencites` |
| `footcites` | `Footcites` |
| `footcitetexts` | `Footcitetexts` |
| `smartcites` | `Smartcites` |
| `textcites` | `Textcites` |
| `autocites` | `Autocites` |
| `supercites` | None |

The call can first give an overall description of up to two round brackets, and then repeat "up to two square bracket descriptions + curly bracket citation key list":

```tex
\parencites(global prenote)(global postnote)
  [see][12]{alpha,beta}
  [34]{gamma}\relax
```

Each set of keys is involved in navigation, reference lookup, completion, semantic highlighting, and renaming. Initiating a name change from a subsequent group or `.bib` entry updates all confirmed references, preserving the overall description, individual group descriptions, and other keys. Completion also supports unclosed key lists in subsequent groups; signature help indicate the overall description and the current parameters of each group.

Parentheses and square brackets end at the first unprotected right bracket, and similar left brackets do not increase nesting. White space and comments are allowed between groups; an empty paragraph, plain text, or `\relax` ends the citation sequence. Redundant descriptions, unclosed descriptions, and dynamic keys are not guessed as static citation keys. See the next section for roll number format.

The compiler may not report a missing documentation warning until it has read forward to the next command. Diagnostics checks the entire call and its read-forward position, locating warnings to matching keys; multiple matching positions provide diagnostics and spelling fixes respectively.

## Volume number reference

The following biblatex commands distinguish between volume numbers, page numbers, and citation keys, and support case and asterisks in tables:

| Single citation | Repeated citation |
| --- | --- |
| `volcite` / `Volcite` | `volcites` / `Volcites` |
| `pvolcite` / `Pvolcite` | `pvolcites` / `Pvolcites` |
| `fvolcite` / `Fvolcite` | `fvolcites` / `Fvolcites` |
| `ftvolcite` / `Ftvolcite` | `ftvolcites` / `Ftvolcites` |
| `svolcite` / `Svolcite` | `svolcites` / `Svolcites` |
| `tvolcite` / `Tvolcite` | `tvolcites` / `Tvolcites` |
| `avolcite` / `Avolcite` | `avolcites` / `Avolcites` |

Each group reads according to "optional prenote, required volume number, optional page number, and required citation key list":

```tex
\volcite[see]{2}[12--16]{alpha}
\volcites(global prenote)(global postnote)
  [see]{1}[12]{alpha,beta}
  [see also]{2}[34]{gamma}\relax
```

Volume numbers, page numbers, and prefixes, even if they have the same name as the key, will not be modified by document renaming or spelling repair. Each set of keys supports navigation, hover, reference lookup, completion, highlighting, and cross-file renaming; missing literature warnings are pinpointed to the key, allowing the compiler to report the line number of the next command.

A single command only reads one group; repeated commands allow up to two parentheses for overall description, and subsequent groups must start with a square bracket prefix or a curly bracket volume number. Also recognizes bare ASCII volume numbers or single macro tokens in started groups, such as `\volcite2{alpha}` and `\volcites{1}{alpha}[see also]2{beta}`; volume number macros are not expanded. Naked Unicode, `^^` escapes, and unconfirmed token boundaries are not inferred, and citation keys are grouped using curly braces.

The completion fragment only inserts the two required parameters of volume number and citation key, and retains the existing call and optional description. The signature help track the volume number, page number and each group of keys respectively; if the required parameters have not been entered after filling in the description or volume number, the next required parameter will continue to be prompted. Ordinary multiple sets of references also support the wait prompt after the description.



## Callback code

Common `AtEveryBibitem`, `AtBeginBibliography`, `AtEveryCite`, `AtEveryCitekey`, `AtEveryMultiCite` and next/current reference callbacks provide code snippets and signature help; static labels, citation keys and category names in the code can be completed, navigated and renamed normally, and calls after the callbacks remain independent. Reference partitions, abbreviation lists, driver and volume reference callbacks are also supported.

The optional entry type of `AtDataInput[entry type]{code}` and the first parameter of `AtBeginBiblist{bibliography list}{code}`/`AtEveryBiblistitem` are processed as selector data, and the remaining parameters are treated as codes. The three parameters of `AtEveryEntrykey{code}{success}{failure}` all preserve code semantics; the asterisk form of `AtUsedriver` and `AtVolcite` preserve the native behavior of the replacement callback. Completion that omits optional parameters will not change the default value, and the code that has already been called will remain intact.

The optional type of `AtDataInput` provides the same common output type completion and suspension instructions as `ifentrytype`. Completion replaces the full name, supports uncompleted headers, comments, CRLF and UTF-16, preserving subsequent code; the same applies to static naming and pure forwarding calls. Built-in type candidates are not provided when parameter data, literal content, unknown capture copies, user overrides, and shared consumers have inconsistent meanings. Skip when the project index is incomplete or the cumulative analysis exceeds 32 MiB, 200000 commands, and 8192 statements.

Static named calls use the same signature as unique pure forwarding aliases; user redefinitions, argument data passed to other macros, and shared consumer ambiguities continue to be protected by project editors. It does not simulate the execution number, sequence or dynamic expansion of callbacks, and does not predict the data of the current entry through the source code.
