"""Source translations for the controlled Draft 0.4 benchmark fixtures.

This is not a Flux source migrator. The strings are authored in compare.py and
differential.py and have one entry, a unit tail, and at most one CSV read.
"""


def fallible_io(source):
    if "! io.print" not in source and "! csv.read" not in source:
        return source
    prefix, entry = source.split("!{", 1)
    body, suffix = entry.rsplit("};", 1)
    if "! csv.read" in body:
        body = body.replace("! csv.read", "^? ! csv.read", 1)
        translated = []
        for line in body.splitlines(keepends=True):
            if "! io.print" in line:
                expression, whitespace = line.rstrip().rsplit(";", 1)[0], line[len(line.rstrip()):]
                line = expression.replace("! io.print", "^= ! io.print", 1) + " { _ : (); };" + whitespace
            translated.append(line)
        body = "".join(translated)
    else:
        body = body.replace("! io.print", "^? ! io.print")
    return prefix + "!{ ^{ " + body + "\n    ()\n} };" + suffix
