export type AvatarGenerationFailureCategory =
  | 'unreachable'
  | 'timeout'
  | 'provider'
  | 'unusable'
  | 'unknown';

export type AvatarGenerationFailure = {
  category: AvatarGenerationFailureCategory;
  message: string;
};

export type AvatarGenerationPhase = 'choosing' | 'generating' | 'candidate' | 'failed';

export type AvatarGenerationFlow = {
  phase: AvatarGenerationPhase;
  currentAvatarDataUrl: string;
  candidateAvatarDataUrl: string | null;
  requestId: string | null;
  failure: AvatarGenerationFailure | null;
};

export type AvatarGenerationOutcome =
  | { status: 'success'; avatarDataUrl: string }
  | { status: 'failure'; failure: AvatarGenerationFailure }
  | { status: 'cancelled' }
  | { status: 'superseded' };

export type AvatarGenerationOperation = {
  epoch: number;
  requestId: string;
};

export function createAvatarGenerationFlow(currentAvatarDataUrl: string): AvatarGenerationFlow {
  return {
    phase: 'choosing',
    currentAvatarDataUrl,
    candidateAvatarDataUrl: null,
    requestId: null,
    failure: null,
  };
}

export function beginAvatarGeneration(
  flow: AvatarGenerationFlow,
  requestId: string,
): AvatarGenerationFlow {
  if (flow.phase === 'generating') {
    return flow;
  }
  return {
    ...flow,
    phase: 'generating',
    requestId,
    failure: null,
  };
}

export function resolveAvatarGeneration(
  flow: AvatarGenerationFlow,
  requestId: string,
  outcome: AvatarGenerationOutcome,
): AvatarGenerationFlow {
  if (flow.phase !== 'generating' || flow.requestId !== requestId) {
    return flow;
  }
  switch (outcome.status) {
    case 'success': {
      const avatarDataUrl = outcome.avatarDataUrl.trim();
      if (!avatarDataUrl) {
        return {
          ...flow,
          phase: 'failed',
          requestId: null,
          failure: avatarGenerationFailure('unusable'),
        };
      }
      return {
        ...flow,
        phase: 'candidate',
        candidateAvatarDataUrl: avatarDataUrl,
        requestId: null,
        failure: null,
      };
    }
    case 'failure':
      return {
        ...flow,
        phase: 'failed',
        requestId: null,
        failure: outcome.failure,
      };
    case 'cancelled':
    case 'superseded':
      return {
        ...flow,
        phase: 'choosing',
        requestId: null,
        failure: null,
      };
  }
}

export function cancelAvatarGeneration(
  flow: AvatarGenerationFlow,
  requestId: string | null = null,
): AvatarGenerationFlow {
  if (
    flow.phase !== 'generating'
    || (requestId !== null && flow.requestId !== requestId)
  ) {
    return flow;
  }
  return {
    ...flow,
    phase: 'choosing',
    requestId: null,
    failure: null,
  };
}

export function changeAvatarStyle(flow: AvatarGenerationFlow): AvatarGenerationFlow {
  if (flow.phase !== 'failed') {
    return flow;
  }
  return {
    ...flow,
    phase: 'choosing',
    requestId: null,
    failure: null,
  };
}

export function acceptAvatarCandidate(flow: AvatarGenerationFlow): {
  flow: AvatarGenerationFlow;
  avatarDataUrl: string | null;
} {
  if (flow.phase !== 'candidate' || !flow.candidateAvatarDataUrl) {
    return { flow, avatarDataUrl: null };
  }
  return {
    avatarDataUrl: flow.candidateAvatarDataUrl,
    flow: {
      ...flow,
      phase: 'choosing',
      currentAvatarDataUrl: flow.candidateAvatarDataUrl,
      requestId: null,
      failure: null,
    },
  };
}

export function ownsAvatarGenerationOperation(
  currentEpoch: number,
  currentRequestId: string | null,
  operation: AvatarGenerationOperation,
): boolean {
  return currentEpoch === operation.epoch && currentRequestId === operation.requestId;
}

export function avatarGenerationFailure(
  category: AvatarGenerationFailureCategory,
  message?: string,
): AvatarGenerationFailure {
  const fallback = {
    unreachable: 'Couldn’t reach the gateway.',
    timeout: 'Avatar generation took too long.',
    provider: 'The image provider couldn’t generate an avatar.',
    unusable: 'The generated image couldn’t be used.',
    unknown: 'Couldn’t generate an avatar.',
  }[category];
  return { category, message: message || fallback };
}
