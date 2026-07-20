// Canonical durable-delivery vocabulary and pure value reducers shared with
// the iOS implementation through spec/conversation-state. This module models
// the protocol only; desktop does not yet claim a process-durable outbox.

export const DURABLE_DELIVERY_STATES = [
  'notDispatched',
  'transportAttempted',
  'ambiguous',
  'acknowledged',
  'cancelledByDiscard',
  'evidence',
  'terminalEvidence',
  'abandoned',
  'supersededByDuplicate',
] as const;
export type DurableDeliveryState = (typeof DURABLE_DELIVERY_STATES)[number];

export const DURABLE_DELIVERY_EVIDENCE = [
  'none',
  'transportAttempted',
  'serverAcknowledged',
] as const;
export type DurableDeliveryEvidence = (typeof DURABLE_DELIVERY_EVIDENCE)[number];

export const DURABLE_DELIVERY_USER_DISPOSITIONS = [
  'none',
  'restoredToDraft',
  'resentAsDuplicate',
  'scopeRevoked',
  'payloadDiscarded',
] as const;
export type DurableDeliveryUserDisposition =
  (typeof DURABLE_DELIVERY_USER_DISPOSITIONS)[number];

export const DURABLE_CREATE_DELIVERY_PHASES = [
  'createPending',
  'threadCreated',
  'bindingCompleted',
  'chatStartAttempted',
  'acknowledged',
  'ambiguous',
] as const;
export type DurableCreateDeliveryPhase =
  (typeof DURABLE_CREATE_DELIVERY_PHASES)[number];

export const DURABLE_CREATE_USER_DISPOSITIONS = [
  'none',
  'restoredToDraft',
  'rebuildMayCreateDuplicateThread',
  'scopeRevoked',
] as const;
export type DurableCreateUserDisposition =
  (typeof DURABLE_CREATE_USER_DISPOSITIONS)[number];

export interface DurableGatewayScope {
  identity: string;
  epoch: number;
}

export interface DurableDeliveryEnvelope {
  text: string;
  attachmentIDs: string[];
  generation: number;
  clientIntentID: string;
}

export interface DurableDeliveryRecord {
  id: string;
  scope: DurableGatewayScope;
  entryID: string;
  reservationID: number;
  correlationID: string;
  envelope?: DurableDeliveryEnvelope;
  state: DurableDeliveryState;
  evidence: DurableDeliveryEvidence;
  userDisposition: DurableDeliveryUserDisposition;
  duplicateRecordID?: string;
}

export interface DurablePayloadConflictCandidate {
  entryID: string;
  label: string;
}

export interface DurablePayloadConflictSet {
  id: string;
  scope: DurableGatewayScope;
  candidates: DurablePayloadConflictCandidate[];
  pendingDecision: boolean;
}

export type DurableDraftRecoveryResult =
  | {
      disposition: 'restored';
      record: DurableDeliveryRecord;
      conflictSet: DurablePayloadConflictSet;
      envelope: DurableDeliveryEnvelope;
    }
  | {
      disposition:
        | 'rejectedNotAmbiguous'
        | 'rejectedConflictScope'
        | 'rejectedConflictDurability';
      record: DurableDeliveryRecord;
      conflictSet: DurablePayloadConflictSet;
    };

export type DurableEvidenceIngressDisposition =
  | 'updated'
  | 'rejectedAuthenticationSource'
  | 'rejectedPhase'
  | 'ambiguousCorrelation'
  | 'unknownCorrelation';

export interface DurableEvidenceIngressResult {
  disposition: DurableEvidenceIngressDisposition;
  records: Record<string, DurableDeliveryRecord>;
  recordID?: string;
}

export interface DurableCreateDeliveryState {
  scope: DurableGatewayScope;
  createIntentID: string;
  entryID?: string;
  threadID?: string;
  phase: DurableCreateDeliveryPhase;
  ambiguousAfter?: DurableCreateDeliveryPhase;
  userDisposition: DurableCreateUserDisposition;
}

function scopesEqual(left: DurableGatewayScope, right: DurableGatewayScope): boolean {
  return left.identity === right.identity && left.epoch === right.epoch;
}

function cloneEnvelope(
  envelope: DurableDeliveryEnvelope | undefined,
): DurableDeliveryEnvelope | undefined {
  if (!envelope) {
    return undefined;
  }
  return { ...envelope, attachmentIDs: [...envelope.attachmentIDs] };
}

function cloneRecord(record: DurableDeliveryRecord): DurableDeliveryRecord {
  return {
    ...record,
    scope: { ...record.scope },
    envelope: cloneEnvelope(record.envelope),
  };
}

function cloneRecords(
  records: Record<string, DurableDeliveryRecord>,
): Record<string, DurableDeliveryRecord> {
  return Object.fromEntries(
    Object.entries(records).map(([id, record]) => [id, cloneRecord(record)]),
  );
}

export function initialDurableDeliveryRecord(input: {
  id: string;
  scope: DurableGatewayScope;
  entryID: string;
  reservationID: number;
  correlationID: string;
  envelope: DurableDeliveryEnvelope;
}): DurableDeliveryRecord {
  if (!input.correlationID || !input.envelope.clientIntentID) {
    throw new Error('durable delivery correlation identifiers must not be empty');
  }
  return {
    ...input,
    scope: { ...input.scope },
    envelope: cloneEnvelope(input.envelope),
    state: 'notDispatched',
    evidence: 'none',
    userDisposition: 'none',
  };
}

export function markDurableTransportAttempted(
  record: DurableDeliveryRecord,
): DurableDeliveryRecord | null {
  if (record.state !== 'notDispatched') {
    return null;
  }
  return {
    ...cloneRecord(record),
    state: 'transportAttempted',
    evidence: 'transportAttempted',
  };
}

export function markDurableDeliveryAmbiguous(
  record: DurableDeliveryRecord,
): DurableDeliveryRecord | null {
  if (record.state !== 'transportAttempted') {
    return null;
  }
  return { ...cloneRecord(record), state: 'ambiguous' };
}

export function acknowledgeDurableDelivery(
  record: DurableDeliveryRecord,
): DurableDeliveryRecord {
  return {
    ...cloneRecord(record),
    state:
      record.userDisposition === 'none' && record.state !== 'terminalEvidence'
        ? 'acknowledged'
        : record.state,
    evidence: 'serverAcknowledged',
    envelope: undefined,
  };
}

export function restoreDurableDeliveryDraft(input: {
  record: DurableDeliveryRecord;
  conflictSet: DurablePayloadConflictSet;
  candidate: DurablePayloadConflictCandidate;
  membershipDurabilityAvailable: boolean;
  allowingUndispatched?: boolean;
}): DurableDraftRecoveryResult {
  const { record, conflictSet } = input;
  const eligible = record.state === 'ambiguous'
    || (input.allowingUndispatched === true && record.state === 'notDispatched');
  if (!eligible || record.userDisposition !== 'none' || !record.envelope) {
    return { disposition: 'rejectedNotAmbiguous', record, conflictSet };
  }
  if (!scopesEqual(conflictSet.scope, record.scope)) {
    return { disposition: 'rejectedConflictScope', record, conflictSet };
  }
  if (!input.membershipDurabilityAvailable) {
    return { disposition: 'rejectedConflictDurability', record, conflictSet };
  }

  const candidates = conflictSet.candidates.some(
    (candidate) => candidate.entryID === input.candidate.entryID,
  )
    ? [...conflictSet.candidates]
    : [...conflictSet.candidates, { ...input.candidate }];
  const envelope = cloneEnvelope(record.envelope);
  if (!envelope) {
    return { disposition: 'rejectedNotAmbiguous', record, conflictSet };
  }
  return {
    disposition: 'restored',
    record: {
      ...cloneRecord(record),
      state: 'abandoned',
      userDisposition: 'restoredToDraft',
      envelope: undefined,
    },
    conflictSet: {
      ...conflictSet,
      scope: { ...conflictSet.scope },
      candidates,
      pendingDecision: true,
    },
    envelope,
  };
}

export function resendDurableDeliveryAsDuplicate(input: {
  record: DurableDeliveryRecord;
  newRecordID: string;
  newClientIntentID: string;
  allowingUndispatched?: boolean;
}): { original: DurableDeliveryRecord; duplicate: DurableDeliveryRecord } | null {
  const { record } = input;
  const eligible = record.state === 'ambiguous'
    || (input.allowingUndispatched === true && record.state === 'notDispatched');
  if (
    !eligible
    || record.userDisposition !== 'none'
    || !record.envelope
    || !input.newRecordID
    || input.newRecordID === record.id
    || !input.newClientIntentID
    || input.newClientIntentID === record.envelope.clientIntentID
  ) {
    return null;
  }

  const envelope = {
    ...record.envelope,
    attachmentIDs: [...record.envelope.attachmentIDs],
    clientIntentID: input.newClientIntentID,
  };
  return {
    original: {
      ...cloneRecord(record),
      state: 'supersededByDuplicate',
      userDisposition: 'resentAsDuplicate',
      duplicateRecordID: input.newRecordID,
      envelope: undefined,
    },
    duplicate: initialDurableDeliveryRecord({
      id: input.newRecordID,
      scope: record.scope,
      entryID: record.entryID,
      reservationID: record.reservationID,
      correlationID: input.newClientIntentID,
      envelope,
    }),
  };
}

export function acknowledgeDurableDeliveryEvidence(input: {
  correlationID: string;
  authenticatedScope: DurableGatewayScope;
  records: Record<string, DurableDeliveryRecord>;
}): DurableEvidenceIngressResult {
  const entries = Object.entries(input.records);
  const matching = entries.filter(([, record]) => (
    record.correlationID === input.correlationID
      && scopesEqual(record.scope, input.authenticatedScope)
  ));
  if (matching.length === 0) {
    const disposition = entries.some(([, record]) => (
      record.correlationID === input.correlationID
    ))
      ? 'rejectedAuthenticationSource'
      : 'unknownCorrelation';
    return { disposition, records: cloneRecords(input.records) };
  }
  if (matching.length !== 1) {
    return { disposition: 'ambiguousCorrelation', records: cloneRecords(input.records) };
  }

  const [recordID, record] = matching[0];
  if (record.state === 'notDispatched' || record.state === 'cancelledByDiscard') {
    return { disposition: 'rejectedPhase', records: cloneRecords(input.records) };
  }
  const records = cloneRecords(input.records);
  if (record.state !== 'acknowledged' && record.state !== 'terminalEvidence') {
    records[recordID] = acknowledgeDurableDelivery(record);
  }
  return { disposition: 'updated', records, recordID };
}

export function settleDurableDeliveryForScopeRevoke(
  record: DurableDeliveryRecord,
): DurableDeliveryRecord {
  let state = record.state;
  let userDisposition = record.userDisposition;
  switch (record.state) {
    case 'notDispatched':
      state = 'cancelledByDiscard';
      userDisposition = 'scopeRevoked';
      break;
    case 'transportAttempted':
    case 'ambiguous':
      state = 'evidence';
      userDisposition = 'scopeRevoked';
      break;
    case 'acknowledged':
      state = 'terminalEvidence';
      if (userDisposition === 'none' || userDisposition === 'payloadDiscarded') {
        userDisposition = 'scopeRevoked';
      }
      break;
    case 'cancelledByDiscard':
    case 'evidence':
    case 'terminalEvidence':
    case 'abandoned':
    case 'supersededByDuplicate':
      if (userDisposition === 'none' || userDisposition === 'payloadDiscarded') {
        userDisposition = 'scopeRevoked';
      }
      break;
  }
  return {
    ...cloneRecord(record),
    state,
    userDisposition,
    envelope: undefined,
  };
}

export function initialDurableCreateDelivery(input: {
  scope: DurableGatewayScope;
  createIntentID: string;
  entryID?: string;
}): DurableCreateDeliveryState {
  if (!input.createIntentID) {
    throw new Error('create intent identifier must not be empty');
  }
  return {
    ...input,
    scope: { ...input.scope },
    phase: 'createPending',
    userDisposition: 'none',
  };
}

export function markDurableCreateThreadCreated(
  state: DurableCreateDeliveryState,
  threadID: string,
): DurableCreateDeliveryState {
  if (state.phase !== 'createPending' || !threadID) {
    return state;
  }
  return { ...state, threadID, phase: 'threadCreated' };
}

export function markDurableCreateBindingCompleted(
  state: DurableCreateDeliveryState,
): DurableCreateDeliveryState {
  return state.phase === 'threadCreated'
    ? { ...state, phase: 'bindingCompleted' }
    : state;
}

export function markDurableCreateChatStartAttempted(
  state: DurableCreateDeliveryState,
): DurableCreateDeliveryState {
  return state.phase === 'threadCreated' || state.phase === 'bindingCompleted'
    ? { ...state, phase: 'chatStartAttempted' }
    : state;
}

export function markDurableCreateResponseLost(
  state: DurableCreateDeliveryState,
): DurableCreateDeliveryState {
  if (state.phase === 'acknowledged' || state.phase === 'ambiguous') {
    return state;
  }
  return { ...state, phase: 'ambiguous', ambiguousAfter: state.phase };
}

export function acknowledgeDurableCreate(
  state: DurableCreateDeliveryState,
): DurableCreateDeliveryState {
  const canAcknowledge = state.phase === 'chatStartAttempted'
    || (state.phase === 'ambiguous' && state.ambiguousAfter === 'chatStartAttempted');
  return canAcknowledge
    ? { ...state, phase: 'acknowledged', ambiguousAfter: undefined }
    : state;
}

export function restoreDurableCreateToDraft(
  state: DurableCreateDeliveryState,
): DurableCreateDeliveryState | null {
  if (state.phase !== 'ambiguous' || state.userDisposition !== 'none') {
    return null;
  }
  return { ...state, userDisposition: 'restoredToDraft' };
}

export function rebuildDurableCreateWithDuplicateRisk(
  state: DurableCreateDeliveryState,
  newCreateIntentID: string,
): { original: DurableCreateDeliveryState; replacement: DurableCreateDeliveryState } | null {
  if (
    state.phase !== 'ambiguous'
    || state.userDisposition !== 'none'
    || !newCreateIntentID
    || newCreateIntentID === state.createIntentID
  ) {
    return null;
  }
  return {
    original: { ...state, userDisposition: 'rebuildMayCreateDuplicateThread' },
    replacement: initialDurableCreateDelivery({
      scope: state.scope,
      createIntentID: newCreateIntentID,
      entryID: state.entryID,
    }),
  };
}

export function settleDurableCreateForScopeRevoke(
  state: DurableCreateDeliveryState,
): DurableCreateDeliveryState {
  const terminal = state.phase === 'acknowledged'
    || (state.phase === 'ambiguous' && state.userDisposition !== 'none');
  if (terminal) {
    return state;
  }
  return {
    ...state,
    phase: 'ambiguous',
    ambiguousAfter: state.phase === 'ambiguous' ? state.ambiguousAfter : state.phase,
    userDisposition: 'scopeRevoked',
  };
}
