# Length diagnostics and fixes

[Back to documentation index](README.md) · [Register and length](registers.md)

When etoolbox reports `Length '\measuer' undefined`, the server will check the complete `deflength` call and log source code echo, accurately mark the length control sequence, and provide spelling fixes. For example:

```tex
\newlength{\measure}
\deflength{\measuer}{2pt+3pt}
```

After selecting ``Replace with length `\measure` ``, the second line becomes `\deflength{\measure}{2pt+3pt}`. Support for bare control sequences, brace-wrapped control sequences, cross-line expressions, and multiple calls on the same line; fixed expressions and assignment prefixes.

## Candidates and boundaries

Spelling suggestions come from standard-length and static `newlength`, `newskip`, `newdimen`, `skipdef`, `dimendef` declarations in the current project, and also recognize literal `csname` declarations. Ordinary macros, count/box/muskip registers, duplicate declarations, and names covered by other types are not length candidates; declarations from other papers are not included in the fix.

Retain the entire line of errors when macro expansion, dynamic or incomplete operands, commands are redefined, or log echo cannot be uniquely confirmed. Precise diagnostics records the macro source when the error occurs, and provides rechecking of source code, candidate types and parameter boundaries before repair; supports versioned editing and old clients, and expired repairs will be rejected.

## Protocol and Verification

The precise diagnostics comes with the `undefined-length` code and name data, such as `{"length":"measuer"}`. For general source verification, see [Build diagnostics](diagnostics.md#analysis-scope-and-source-confirmation). For default, protocol and real three-engine regression commands, see [Test Instructions](testing.md#undefined-length-diagnostics).
