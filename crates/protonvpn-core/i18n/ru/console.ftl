# The collapsed console bar's leftmost button while the transcript is open: it folds the pane back
# down to one line. Short — it shares a dense row with the runner status and the state.
console-collapse = Свернуть

# The same button while the transcript is closed: it opens the pane. The pane is called "the
# transcript" throughout this file; the word is the button's label and the caption's first word.
console-transcript = Транскрипт

# The button at the far right of the console bar that interrupts the command the runner is
# executing. It is visible only while something runs, and it is the way out of a command waiting
# for something that will never come. Short, and drawn in the danger colour.
console-cancel = Прервать

# The remark at the far right of the console bar when nothing is running, in place of the Cancel
# button. An eyebrow: the widget draws it in capitals. It says the pane is a record, not a prompt.
console-read-only = консоль только для чтения

# The caption at the left of the control row above the transcript blocks. Lowercase and faint.
# "verbatim" is the promise this pane exists to keep: it shows the CLI's bytes, not our summary.
console-transcript-caption = транскрипт · вывод CLI показан дословно

# The button in that control row that scrolls the transcript to its end. Short: the row also
# carries Copy all, and the transcript scrolls away from the bottom while new output arrives.
console-bottom = Вниз

# The button beside it that copies the whole transcript to the clipboard. It copies the CLI's
# output as it arrived, including the command lines.
console-copy-all = Копировать всё

# The faint line above the transcript when the log bus had to throw away the oldest invocations to
# stay inside its buffer. The leading ellipsis is deliberate and stays: it says something is
# missing above. $count is how many invocations were dropped — a number, not data.
console-dropped = { $count ->
        [one] … { $count } более ранний вызов вытеснен из буфера
        [few] … { $count } более ранних вызова вытеснено из буфера
        [many] … { $count } более ранних вызовов вытеснено из буфера
       *[other] … { $count } более ранних вызовов вытеснено из буфера
    }

# Shown instead of the transcript when nothing has run yet. "verbatim" again: it is what the pane
# promises before it has anything to show.
console-empty = Пока ничего не запускалось. Каждая команда появится здесь дословно.

# The copy button at the right of one invocation's header, beside that invocation's command line.
# It copies this one invocation, not the whole transcript. Short: the header is a single monospace
# line and the command itself takes the room.
console-copy = Копировать

# The faint line inside one invocation's block when the middle of an oversized output was dropped
# to keep the console readable. As above, the leading ellipsis is deliberate: lines are missing.
# $count is how many lines are not shown — a number, not data.
console-lines-hidden = { $count ->
        [one] … { $count } строка скрыта (лимит показа)
        [few] … { $count } строки скрыто (лимит показа)
        [many] … { $count } строк скрыто (лимит показа)
       *[other] … { $count } строк скрыто (лимит показа)
    }
