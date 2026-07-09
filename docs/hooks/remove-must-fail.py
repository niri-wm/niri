from __future__ import annotations
import re

# todo: this could be done generically, so that any 
# ```language,annotation,anything-else 
# is reduced to
# ```language
# which is what's supported by mkdocs/pygments
# also note: mkdocs provides ways to highlight lines, add line numbers
# but these are added as 
# ```language linenums="1"
# and not split by comma
def on_page_markdown(
    markdown: str, *, page, config, files
):
    return re.sub(
        r",must-fail",
        '', markdown, flags = re.I | re.M
    )