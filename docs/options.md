# Contextual option completion

Option names and common enumeration values can be completed in the optional parameters of `[options]`, `includegraphics` of the package or document class, the setting group of `geometry` / `newgeometry` / `hypersetup`, and known families such as `setkeys{Gin}`. Optional parameters for `itemize`, `enumerate`, and `description` provide common enumitem options.

For example, `\usepackage[mar]{geometry}` can be supplemented by `margin=`; `\includegraphics[keepaspectratio=t]{figure}` can be supplemented by `true`. Supports geometry, graphicx, hyperref and common document class options. Existing options will not be repeatedly recommended. Commas within curly braces will not separate outer options. Modifying the option name will retain the existing `=value`. Editing ranges are calculated in UTF-16.

These are a selection of static options data. User-redefined commands with the same name, unknown packages, dynamic families, or fragments in comments will not apply this option list; the server will not execute or derive any custom keys from the package source code.
