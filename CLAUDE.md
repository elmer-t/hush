## Workflow
- Work only from GitHub issues labelled `ready-for-agent`. On start, swap the label for `in-progress`.
- Order: skip an issue whose body says `Depends on #<n>` while any of those issues is still open. Among the rest, take the lowest number first.
- Branch name: `issue-<n>-<slug>`. Open a PR with `Closes #<n>`.
- Blocked on a decision? Add `ready-for-human`, comment the question on the issue, and stop.