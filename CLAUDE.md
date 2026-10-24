## Workflow
- Work only from GitHub issues labelled `ready-for-agent`. On start, swap the label for `in-progress`.
- Order: skip an issue whose body says `Depends on #<n>` while any of those issues is still open. Among the rest, take the lowest number first.
- Branch name: `issue-<n>-<slug>`. Open a PR with `Closes #<n>`.
- Don't stop for a decision. Make the call yourself, keep working, and list each call with its reasoning under a `Decisions` heading in the PR body so it can be overridden in review.
- Pause only before something that can't be undone: a change that could leave existing vaults unreadable (schema, encryption or KDF settings without a migration), deleting user data, or force-pushing a shared branch. Then add `ready-for-human`, comment the question on the issue, and stop.
