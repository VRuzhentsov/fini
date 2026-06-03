# Focus Reminder Preemption E2E

## Contract

Computed Focus treats active reminder due timestamps as virtual Focus events.
For active quests, `quest.due + quest.due_time` competes with persisted
`focus_history.created_at` rows such as manual Set Focus. The youngest valid
timestamp wins.

## Scenario

1. Open the app on Focus.
2. Create or select a current Focus quest.
3. Create a new active quest with a reminder due in the near future.
4. Immediately after saving the reminder, verify current Focus has not changed.
5. Wait until the reminder due time arrives.
6. Verify the reminder quest becomes Focus while the app remains open.

## Expected State Chain

Before due time:

- Manual `focus_history.created_at` is valid.
- Future reminder timestamp is ignored because it has not arrived.
- `get_active_focus` returns the manual Focus quest.
- Focus UI shows the manual Focus quest.

After due time:

- Reminder timestamp is now valid.
- Reminder timestamp is newer than the manual Focus timestamp.
- `get_active_focus` returns the reminder quest.
- Focus UI shows the reminder quest without requiring app restart.

## Evidence

- Write path: create quest and save `due` + `due_time` through the app UI.
- Persistence path: backend stores the quest due fields and derived reminder row.
- Read path: `get_active_focus` returns the newest valid candidate.
- UI path: Focus active card changes after the due boundary.

## Non-Goals

- This test does not prove native Android notification shade delivery.
- This test does not require a persisted `trigger = reminder` FocusHistory row.
- This test does not cover snooze behavior.
