# Kernel hooks

[Back to documentation](README.md)

Standard kernel hooks provide command completion, snippets, hover instructions, and signature help, including direct calls, static `csuse`/`csname`, and pure forwarding aliases. Redefinition within the project uses user signature, and unsaved dependency updates take effect immediately.

| Interface | Parameters |
| --- | --- |
| `AddToHook`、`AddToHookWithArguments` | `{hook}[label]{code}` |
| `AddToHookNext`、`AddToHookNextWithArguments` | `{hook}{code}` |
| `AtBeginDocument`、`AtEndDocument` | `[label]{code}` |
| `RemoveFromHook` | `{hook}[label]` |
| `NewHook`、`NewReversedHook`、`UseHook`、`UseOneTimeHook` | `{hook}` |
| `UseHookWithArguments`, `UseOneTimeHookWithArguments` | `{hook}{argument count}`, followed by hook data parameters |
| `IfHookEmptyTF`、`IfHookExistsTF` | `{hook}{true code}{false code}` |
| `IfHookEmptyT` | `{hook}{true code}` |
| `IfHookEmptyF` | `{hook}{false code}` |
| `NewHookWithArguments`、`NewReversedHookWithArguments` | `{hook}{argument count}` |
| `NewMirroredHookPair` | `{before hook}{after hook}` |
| `NewMirroredHookPairWithArguments` | `{before hook}{after hook}{argument count}` |
| `DeclareHookRule` | `{hook}{first label}{relation}{second label}` |
| `DeclareDefaultHookRule` | `{first label}{relation}{second label}` |
| `ClearHookRule` | `{hook}{first label}{second label}` |
| `ClearHookNext`、`ActivateGenericHook`、`DisableGenericHook`、`ShowHook`、`LogHook` | `{hook}` |
| `SetDefaultHookLabel`、`PushDefaultHookLabel` | `{label}` |
| `PopDefaultHookLabel`, `DebugHooksOn`, `DebugHooksOff` | No parameters |

Fragments omit optional tags and retain existing arguments. Tag references in full code parameters preserve structural navigation and renaming; names, tags, number of parameters, and ordering are protected by data. Unclosed parameters continue to protect the internal token, and the independent source code after the complete call participates in analysis normally.

Code parameters are treated as deferred codes and do not infer the current document partition, classification or citation context, nor are they directly merged with the environment and mathematical status of the text. Message or literal readers in code do not authenticate the execution identity of tags within them; maintain structural indexing consistent with the deferred macro body.

`UseHook`/`UseOneTimeHook` will also read the declared parameters, not only the interface with `WithArguments`. Parameters may be used as literal text by hooks: if a macro or reference rename would involve these data tokens, the server rejects the entire rename, avoiding changes to captured control sequence names or displayed reference keys. Protection covers static naming, pure forwarding and copying calls, unsaved declarations across files, and different consumers of shared source code. Dynamic quantities, declaration ambiguities, and one-time calls use conservative parameter upper bounds; this protection does not prove that the command in the parameter will be executed. Independent labels can still be renamed, and there is an upper limit on the amount of analysis workload.

Parameter protection is also passed along with the wrapped macro: the actual parameters of e.g. `\newcommand\invoke[1]{\UseHook{custom}{#1}}` may become literal data. Analysis covers legacy/xparse parameters, optional defaults, nested wrappers, and possibly retaining copies of the old body; independent parameters that do not enter the data continue to participate in full renaming. Naked `#1` can be replaced by empty, so it will also check whether the hook continues to read the token after the wrapped call; TeX fixed prefixes and delimiters belong to the calling syntax and are not counted as actual data. If the parameter syntax cannot be confirmed or the workload exceeds the limit, the name change will be conservatively refused.

The same applies to traditional and xparse environment packaging. Formal parameters in the start body, start parameters referenced by xparse's end body, and optional default values are all mapped to actual parameters of `begin`; support for named/purely forwarded environment interfaces, nesting of commands and environments, and environment duplication that may preserve old parameter syntax. Shared files must be considered for all consumers. Unsaved text and default value updates take effect immediately; independent formal parameters and labels remain fully edited.

A wrapper that consumes its own parameters first and then forwards only `UseHook` or environment interfaces is not equivalent to a zero-parameter alias. For example, `\def\invoke#1{\UseHook}` will consume `#1` first, and the hook name will be read later; the complete boundaries of this type of additional syntax are not currently certified, and name changes are refused when related calls are encountered. Dynamic environment names and incomplete parameters also use conservative bounds, which do not prove the actual execution of the environment or hook.

Environment declarations also generate start/stop control sequences that can be called directly. `\invoke{Ignored}{custom}\word` of `\newenvironment{invoke}[1]{\UseHook}{}` will also consume its own parameters first; the security graph covers these control sequences, static naming, nested forwarding and command/environment replication. Zero parameters start or end the control sequence and read the protected data according to the possible interface. Forwarding with extra parameters remains rejected. Changes in the meaning of unsaved and shared consumers take effect immediately. These security edges are not used to authenticate read-only interactions or perform identities on the fly.

When the registered code may redefine the string judgment command, revoke the subsequent known meaning of the relevant command to avoid treating the real macro call as literal data and missing changes after the hook is executed. This protection does not assume that the hook has been unused; it may still be triggered again after an explicit restore, so the restore command will not clear the undetermined impact of the hook. Unconfirmed file loads and out-of-limit analysis are also handled conservatively, and individual tags remain editable.

The complete branch of the conditional interface preserves structure references and label renames, but does not authenticate the actual selected branch, nor is it used to prove that the code is executed immediately. Potential assignments, macro patch callbacks, and their static naming/forwarding/copying calls in either branch are protected by string meaning to avoid overwriting only part of the actual macro call. Recovery in the unselected branch will not offset potential modifications in another branch; irrelevant modifications and conditions in uncalled definitions will not affect the original string judgment.

## Hook name

Literal hook names support definition jump, reference search, suspension, highlighting and name completion. Definitions are provided by `NewHook`, `NewReversedHook` and corresponding declarations with parameters/mirrors; the names of registration, usage, conditions, removal, sorting and viewing interfaces are associated in independent namespaces and are not mixed with labels, macros or code labels of the same name. Completion replaces the full suffix of a name, supporting empty names, unclosed groupings, and UTF-16/CRLF coordinates; candidates come from the full literal declaration of the current project.

Direct calls, static `csuse`/`csname` and unique pure forwarding aliases use the same index. Unsaved definitions across files are effective immediately; shared source code only provides interaction when all consumers agree on the meaning of the call. The full deferred context preserves source code navigation, confirmed data arguments and replaced kernel interfaces without creating hook references. This is source code structure navigation and does not prove that the hook has been declared or triggered.

Name matching preserves kernel differences: the declaration/registration interface normalizes the normal TeX whitespace at the beginning and end of the name, `UseHook`/`UseHookWithArguments` and the four conditional interfaces preserve the original name. `NewHook{ custom }` can be hit by `UseHook{custom}`, while `UseHook{ custom }` is not associated with it. The dots in common names remain literal. `.`/`./...` names that contain internal whitespace, rely on default tags, dynamic expansion and cross-annotation splicing are not inferred; there are no built-in or automatically generated hooks declared in the source code and no fictitious definition locations.

Custom literal names support prepared renaming and complete cross-file renaming. A clear source code declaration is required, and all shared consumers recognize the same declaration and interface meaning; name positions such as declaration, registration, usage, conditions, and sorting will be updated at the same time, and the other name of the mirror declaration remains independent. Reference tags, macros, code tags and ordinary text with the same name retain their original meaning. The leading and trailing blanks and original comments of the declaration/registration are retained; when the bare token name is changed to multiple characters, curly braces are added to avoid splitting the new name into multiple parameters. Unsaved provides instant refresh of changes in files and consumers, supports two WorkspaceEdit formats and the version number of the open document.

Dynamic or relative names, incomplete fields, unknown number of parameters, identity differences between different consumers, capture duplication, and interface forwarding that consumes its own parameters first cannot authenticate a complete name change. Target names cannot capture existing declarations or uses, nor enter the kernel or automatically triggered namespaces, such as `begindocument`, `label`, `env/...`, `cmd/...`, `file/...`, `shipout/...`, and reserved names that hold global default rules. `??`. Incomplete source indexing, unknown file loading, and executable literature database codes are also rejected for editing.

The first version of name editing only authenticates the above static public interface. Low-level expl3 expansion/copying, internal hook storage access (including legacy `@namehook` code blocks), deprecated interfaces such as `DisableHook`/`ProvideHook`, uncertified named executions such as `UseName`, dynamic control sequence declarations, related macro patches, and unknown undefined targets may hide or alter the actual call, so are conservatively rejected. Explicit string execution, character case conversion and catcode modification do not participate in name editing authentication: for example, `\lowercase{\UseHook{CUSTOM}}` will call `custom`, and cannot be directly renamed according to source code case. Source code redefinition or related patches of literal readers (including `\detokenize`, verbatim, URL) also refuse authentication to avoid leaking executable calls hidden by the parser from the edit set. Denying name editing does not undo existing read-only navigation; these restrictions do not prove that any hooks have been executed. Code tags use independent navigation as described below; code tag name changes are still pending approval.

Use the interface to provide signature help such as `{argument 1}` based on the unique and confirmable literal declaration in the project, and exclude data parameters from the reference navigation. Declared hooks use the declared number, covering the number at the call site; undeclared literal hooks use the 0–9 number given by the `WithArguments` interface. Both direct, static naming and pure forwarding calls are applicable, and unsaved file changes refresh prompts and navigation boundaries are provided. Independent references after declared zero-argument hooks retain normal jumps.

This is still static source code analysis and does not simulate hook triggering, deletion, sorting, or one-time execution. No precise additional signature help is provided when a one-time call is detected, and the data boundary adopts a conservative upper bound of the number of declarations and calls; dynamic names, dynamic declarations, and sources of ambiguity do not authenticate the exact number. The rename check uses a wider security boundary, so some native executable references may still be rejected for rename; signature help cannot be used as proof of execution.

Verify names, conditional interactions, and native behaviors:

```sh
cargo test --locked --test hook_names --test hook_conditions --test hook_arguments --test hook_variadic
cargo test --locked --test hook_names --test hook_conditions --test hook_arguments --test hook_variadic -- --ignored
cargo test --locked --test protocol hook_names_keep_exact_ranges
cargo test --locked --test protocol hook_condition_guards
cargo test --locked --test protocol hook_argument_data_guards
cargo test --locked --test protocol hook_wrapper_data_guards
cargo test --locked --test hook_environments
cargo test --locked --test hook_environments -- --ignored
cargo test --locked --test protocol hook_environment_data_guards
cargo test --locked --test protocol generated_environment_interface_guards
cargo test --locked --test protocol variadic_hook_help
cargo test --locked --test hook_rename
cargo test --locked --test hook_rename -- --ignored
cargo test --locked --test protocol hook_name_renames_keep_exact_namespaces
```

## Explicit code tags

The explicit optional tags of `AddToHook`/`AddToHookWithArguments` support definition jump, reference search, hover, highlighting and completion. The optional tag of `RemoveFromHook`, the two tags of `DeclareHookRule`, and the two tags of `ClearHookRule` use the same index. Identity is determined by `(hook name, code label)`; multiple code registrations for the same tag are returned as definitions and are not merged with other hooks, normal reference keys, or macros.

Explicit tags for `AtBeginDocument[label]{code}` and `AtEndDocument[label]{code}` are grouped into `begindocument` and `enddocument` respectively, associated with the corresponding `AddToHook`, removal and sorting rules; start/end tags with the same name remain independent. When reading the interface meaning that can be confirmed in the project, it is also checked whether the underlying `AddToHook` has been replaced by the source code. When the entry is changed to another confirmed pure forwarding, the current kernel leaf interface and its parameter shape are parsed. For example, the start entry forwarded to `AtEndDocument` is classified as an end hook. Unknown interface or data operand does not create a registered location.

Supports direct, static naming, legacy/xparse pure forwarding, and environment-generated zero-parameter interfaces. Boundary comments and leading and trailing TeX whitespace normalization, cross-comment splicing and dynamic names are not inferred for the time being. Relative fields only support the following partial situations. Required fields allow bare tokens; the first character of adjacent parameters is given priority to the latter field, and is automatically added to the group when it is completed into a multi-character label. Completion replaces complete suffixes, filters by hook and prefix, supports empty fields, unclosed fields and UTF-16/CRLF ranges. Immediate refresh of unsaved provision files and share consumer meaning changes.

The two explicit tags of `DeclareDefaultHookRule` use the global rule scope: the definition jump returns the same-named explicit registration of each hook in the current project, and the reference search includes these registrations, local rules and removal positions; the completion takes the union from the registration tags of each hook and replaces the complete suffix of the current field. Hovering the global rule field indicates that it affects all hooks. The definition jump and completion of local fields are still limited to the own hook, but the reference search will include the global rule of the same name, and the registration of other hooks will not be merged into the local definition.

The kernel stores the global default rules under the reserved name `??`; the confirmed `DeclareHookRule{??}` and `ClearHookRule{??}` labels are also associated with the same global scope, and the TeX blanks at the beginning and end of the name are normalized normally. Local rules can override global sorting. Clearing global rules will change the order of hooks; the index retains all source code locations and does not simulate the final sorting or deletion state accordingly. If the interface is purely forwarded to another confirmed entry, it will be read according to the actual parameter shape and scope of the current leaf interface.

Explicit literal fields of `PushDefaultHookLabel{label}`/`SetDefaultHookLabel{label}` also support definition, citation, hover, highlighting, and completion. Setting the default label can affect different hooks, so jumping from the settings field will return the explicit registration of each hook in the project with the same name, and the completion takes the union of the registration labels of each hook; the reference to the local code label also contains the corresponding setting location. The setting itself is registered as a reference, not as code. Supports confirmable static naming and pure forwarding entries, boundary annotations, bare single tokens, empty and unclosed fields, and complete suffix replacement; this interaction is not generated when the interface is replaced or becomes data.

`top-level` in explicit registration and removal of fields supports independent document-level code slot navigation, referencing, hovering, highlighting and completion. Multiple registrations of the same hook are returned together, and other hooks remain independent; the sorting rules and default settings fields are not associated with or recommended for this slot. The kernel places it after the sorting code in normal hooks and before it in reverse hooks. Normal collation does not change the slot, and explicit removal clears it. Reserved slots do not provide renaming.

The explicit registration/removal field immediately adjacent to the literal `PushDefaultHookLabel{base}` parses `.` and `./suffix`, which are associated with the explicit code blocks of `base` and `base/suffix` respectively. Only TeX blanks and comments are allowed between setting and calling. The interfaces and data roles of both parties must be confirmed. All sharing consumers must identify the same identity. Supports direct, static naming, traditional/xparse and environment-generated pure forwarding entries and life cycle registration; definitions/references use parsed identities, and the scope retains the original relative fields. The completion is still written as `.`/`./suffix`, other bases and hooks are excluded, and the complete suffix is replaced. Sets, Pops, other calls, closed groups, or settings separated by file boundaries are not used for this local inference; nor can a Push in the data or a Push individually deferred by `aftergroup` establish it.

This is still structural navigation and does not simulate registration time, removal or sorted execution states. Omitting tags, broader `.`/`./suffix` execution context, relative collation fields, and `*` have not been authenticated and do not generate corresponding registration navigation. Support for setting fields does not imply that subsequent omission of labels has been inferred: the kernel's default label stack is global, the setting is retained across TeX groups, package/class loading automatically modifies and restores it, and settings in deferred code do not take effect until execution. Code label renaming has not yet been implemented: global default rules may connect multiple hooks, local string replacement will change the sorting, and the read-only definition position cannot be directly reused as a complete edit set. Native regression covers code block order, removal, actual events, full AUX before and after independent reference edits, and four global rule edit controls. Changing a normal custom hook to `??` would conflict with the kernel rules store, so the name change explicitly rejects this goal.

The life cycle project also uses native events to distinguish the start and end phases, and does not register code when the check interface is replaced or called as hook data. The actual two edits to a common reference key with the same name will not change the code label, and the execution order and intact AUX of all code blocks will be preserved after editing. Low-level interface replacement of unsaved provided files, forwarding target switching, shared consumer divergence and recovery all have accurate UTF-16/CRLF stdio regression, and normal tag editing retains the current document version.

The global rule specialization also covers direct, static naming, traditional/xparse and environment-generated control sequence forwarding, required bare fields, dynamic relationship parameters, boundary annotations, empty and unclosed fields. Exact stdio regression for unsaved interface replacement, shared consumer divergence, shutdown recovery, non-BMP characters on the same line, and both edit formats; global tag queries do not trigger renaming of common reference keys with the same name.

The default tag settings specifically cover the consistency of both entries, named/pure forwarding interfaces, data parameters, unsaved updates, and shared consumers. The seven native scenes confirm that they are omitted or the `.` registration is indeed merged into the set tag, and disappear together when the code block is removed; the actual two edits of the common reference key with the same name retain settings, code events, and complete AUX. Exact stdio checks set field ranges, highlighting, and read-only boundaries, as well as the meaning of file switching and closing recovery when the main file version remains unchanged.

```sh
cargo test --locked --test hook_code_labels
cargo test --locked --test hook_code_labels -- --ignored
cargo test --locked --test protocol global_hook_rules_refresh
cargo test --locked --test protocol adjacent_push_relative_hook_labels_refresh
cargo test --locked --test protocol document_level_hook_slots_refresh
cargo test --locked --test protocol default_hook_label_settings_refresh
cargo test --locked --test protocol hook_name_renames_keep_exact
```
