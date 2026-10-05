# The small "back" button in the top right corner of the sign-in card, on the same row as the logo.
# It is shown only when the user opened the sign-in page themselves: it returns to the app without
# signing in. When the CLI says nobody is signed in there is nothing to go back to, and the button
# is not drawn at all. Short — 12pt in a ghost button the width of its own text.
login-back = Back

# The first of the two step markers on the row above the sign-in form. The digit is drawn beside it
# by the view; this is the word only. It is the step the form is on while the username and the
# password are being typed, and it dims once the form has moved on to the two-factor code. Not a
# heading: 12pt inline, and the pair must stay short enough to sit on one row.
login-progress-account = Account

# The second step marker on that row, drawn in the accent colour while the CLI waits for the
# two-factor code. It names the step, not the field — the field's own label is
# login-two-factor-label. 12pt inline.
login-progress-two-factor = Two-factor code

# The title of the sign-in card, the largest line on the page (24pt) and alone on its row. "Proton
# VPN" is the product's name: leave it as it is.
login-title = Sign in to Proton VPN

# The paragraph under the title: the promise this page exists to make. It is a statement of fact
# about how the credentials are handled, not reassurance, and it has to survive translation with
# the same precision — the wrapper really does run `protonvpn signin`, and the password and the
# code really are written into the CLI's PTY, so they never reach the argv and never appear in the
# console transcript below. Two sentences in a card 470px wide. `protonvpn signin` is a command
# name and is never translated.
login-explainer = The wrapper runs protonvpn signin and writes the password and the code straight into the PTY. Secrets never reach the console.

# The label above the username field, 13pt. Proton issues an address rather than a login name;
# "Proton" is the product's name and stays as it is.
login-username-label = Proton username

# The placeholder inside the empty username field: an example of the address Proton expects, not a
# label — it disappears as soon as anything is typed. It is an address, not prose: leave it exactly
# as it is in every language, and do not translate "proton.me".
login-username-placeholder = user@proton.me

# The label above the password field, 13pt.
login-password-label = Password

# The small button beside the password field that reveals what was typed. It names the action the
# click performs, so it is the opposite of login-password-hide. Short: 12pt, in a ghost button
# beside the input.
login-password-show = Show

# The same button while the password is visible: clicking it hides the password again.
login-password-hide = Hide

# The primary button that sends the credentials and moves on to the two-factor step if the account
# has one. It fills the width of the card, 14pt. It does not say "Sign in": the CLI may still ask
# for a code, and the button must not promise an outcome it cannot know yet.
login-submit = Continue

# The faint line at the bottom of the sign-in card, under the form, 11pt. It says where typing goes
# and what the console below will show — that transcript is the product, and this is the one
# sentence that tells the user it will never hold the password. "PTY" is the technical term and
# stays; the "·" is a separator drawn as part of the message.
login-pty-note = Typing goes into the PTY · the transcript below shows only command names and the CLI's output.

# The label above the two-factor code field, 13pt. It replaces the username and password fields
# once the CLI asks for the code, so the card holds one form at a time.
login-two-factor-label = Two-factor authentication code

# The placeholder inside the empty two-factor field: the shape of what the CLI expects, not a
# label. The 6 is a fact about the code, not a count to decline, so it stays a literal in every
# language. Short — the field shares its row with login-two-factor-submit.
login-two-factor-placeholder = 6 digits

# The button beside the two-factor field that sends the code to the waiting CLI process. Short:
# 13pt, in a row with the input, so it must not grow. It is not login-submit over again — that
# button already ran.
login-two-factor-submit = Confirm

# The muted line under the two-factor field, 12pt, in a card 470px wide. The same promise as
# login-explainer, at the step where it matters most: the code goes into the process, not into the
# arguments. "PTY" stays as it is.
login-two-factor-note = The CLI asked for the code in the PTY session — it goes straight into the process and appears neither in the arguments nor in the console.

# The heading of the card beside the sign-in form. The widget draws it in capitals itself, so write
# it in ordinary case. It is the eyebrow over a numbered list of four items saying what signing in
# actually does.
login-steps-title = What happens during sign-in

# Item 1 of the four. The digit is drawn beside it by the view; this is the sentence only.
# `protonvpn signin` is the command that runs: never translate it.
login-steps-1 = protonvpn signin runs in a PTY session.

# Item 2 of the four. The point is where the password goes: into the process's own input stream,
# not into the argv, where any other process on the machine could read it.
login-steps-2 = The password is written into the process's stream, not into the command-line arguments.

# Item 3 of the four. "2FA" is the account's setting, not a count: leave the abbreviation as it is.
login-steps-3 = If the account has 2FA enabled, the CLI asks for a code — the field appears right here.

# Item 4 of the four. The console is where the CLI's output is shown; this says what is not in it.
login-steps-4 = The console keeps only command names and the CLI's output, no secrets.
