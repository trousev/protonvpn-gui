# The collapsed console bar's leftmost button while the transcript is open: it folds the pane back
# down to one line. Short — it shares a dense row with the runner status and the state.
console-collapse = Collapse

# The same button while the transcript is closed: it opens the pane. The pane is called "the
# transcript" throughout this file; the word is the button's label and the caption's first word.
console-transcript = Transcript

# The button at the far right of the console bar that interrupts the command the runner is
# executing. It is visible only while something runs, and it is the way out of a command waiting
# for something that will never come. Short, and drawn in the danger colour.
console-cancel = Cancel

# The remark at the far right of the console bar when nothing is running, in place of the Cancel
# button. An eyebrow: the widget draws it in capitals. It says the pane is a record, not a prompt.
console-read-only = console is read-only

# The caption at the left of the control row above the transcript blocks. Lowercase and faint.
# "verbatim" is the promise this pane exists to keep: it shows the CLI's bytes, not our summary.
console-transcript-caption = transcript · the CLI's output is shown verbatim

# The button in that control row that scrolls the transcript to its end. Short: the row also
# carries Copy all, and the transcript scrolls away from the bottom while new output arrives.
console-bottom = Bottom

# The button beside it that copies the whole transcript to the clipboard. It copies the CLI's
# output as it arrived, including the command lines.
console-copy-all = Copy all

# The faint line above the transcript when the log bus had to throw away the oldest invocations to
# stay inside its buffer. The leading ellipsis is deliberate and stays: it says something is
# missing above. $count is how many invocations were dropped — a number, not data.
console-dropped = { $count ->
        [one] … { $count } earlier invocation dropped from the buffer
       *[other] … { $count } earlier invocations dropped from the buffer
    }

# Shown instead of the transcript when nothing has run yet. "verbatim" again: it is what the pane
# promises before it has anything to show.
console-empty = Nothing has run yet. Every command will appear here verbatim.

# The copy button at the right of one invocation's header, beside that invocation's command line.
# It copies this one invocation, not the whole transcript. Short: the header is a single monospace
# line and the command itself takes the room.
console-copy = Copy

# The faint line inside one invocation's block when the middle of an oversized output was dropped
# to keep the console readable. As above, the leading ellipsis is deliberate: lines are missing.
# $count is how many lines are not shown — a number, not data.
console-lines-hidden = { $count ->
        [one] … { $count } line hidden (display limit)
       *[other] … { $count } lines hidden (display limit)
    }
