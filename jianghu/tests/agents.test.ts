import assert from 'node:assert/strict';
import test from 'node:test';
import { required } from './required.ts';
import { createAgentStore, demoAgents } from '../src/agents.ts';

test('approval requests route oldest first and cannot be resolved by a stale action', () => {
  const store = createAgentStore(demoAgents);
  assert.equal(store.dispatch({ type: 'approval.resolved', id: 'lin', approvalId: 'missing', outcome: 'approved' }), false);
  const first = required(store.pending()[0]);
  store.dispatch({
    type: 'approval.requested',
    id: 'lin',
    approvalId: 'later',
    at: 30,
    summary: 'Check draft.',
  });
  assert.deepEqual(
    store.pending().map((agent) => agent.id),
    [first.id, 'lin'],
  );
  assert.equal(
    store.dispatch({ type: 'approval.resolved', id: first.id, approvalId: 'stale', outcome: 'approved' }),
    false,
  );
  assert.equal(required(store.get(first.id)).status, 'needs_approval');
  store.dispatch({
    type: 'approval.resolved',
    id: first.id,
    approvalId: required(first.approval).id,
    outcome: 'declined',
  });
  assert.equal(required(store.get(first.id)).status, 'waiting');
  assert.equal(store.pending()[0].id, 'lin');
});

test('snapshots and listeners reflect local events in the same order', () => {
  const store = createAgentStore(demoAgents);
  const statuses: string[] = [];
  const unsubscribe = store.subscribe(() => statuses.push(required(store.get('lin')).status));
  store.dispatch({ type: 'agent.status', id: 'lin', status: 'running', activity: 'Checked the files.' });
  store.dispatch({ type: 'agent.status', id: 'lin', status: 'done', activity: 'Draft ready.' });
  unsubscribe();
  assert.deepEqual(statuses, ['running', 'done']);
  assert.equal(required(demoAgents.find((agent) => agent.id === 'lin')).status, 'running');
});
