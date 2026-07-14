import type {
  DesktopCapsuleSummary,
  SetCapsuleFavoriteResult,
} from '@shared/contracts';

export type CapsuleGalleryTab = 'all' | 'favorites';

export interface CapsuleFavoriteMutationState {
  serverFavoritedAt: string | null;
  desiredFavorited: boolean;
  inFlight: boolean;
}

export interface CapsuleFavoritesState {
  favoritesGeneration: number;
  mutations: Record<string, CapsuleFavoriteMutationState>;
}

export interface CapsuleFavoriteEffect {
  capsuleId: string;
  favorited: boolean;
}

export interface CapsuleFavoriteTransition {
  capsules: DesktopCapsuleSummary[];
  state: CapsuleFavoritesState;
  effect: CapsuleFavoriteEffect | null;
}

export function createCapsuleFavoritesState(): CapsuleFavoritesState {
  return { favoritesGeneration: 0, mutations: {} };
}

function serverFavorited(mutation: CapsuleFavoriteMutationState): boolean {
  return mutation.serverFavoritedAt !== null;
}

function initialMutation(capsule: DesktopCapsuleSummary): CapsuleFavoriteMutationState {
  return {
    serverFavoritedAt: capsule.favoritedAt,
    desiredFavorited: capsule.favoritedAt !== null,
    inFlight: false,
  };
}

function replaceCapsule(
  capsules: DesktopCapsuleSummary[],
  replacement: DesktopCapsuleSummary,
): DesktopCapsuleSummary[] {
  return capsules.map((capsule) =>
    capsule.id === replacement.id ? replacement : capsule,
  );
}

export function capsuleIsFavorited(
  capsule: DesktopCapsuleSummary,
  state: CapsuleFavoritesState,
): boolean {
  return state.mutations[capsule.id]?.desiredFavorited ?? capsule.favoritedAt !== null;
}

export function filterCapsulesForGallery(
  capsules: DesktopCapsuleSummary[],
  tab: CapsuleGalleryTab,
  state: CapsuleFavoritesState,
): DesktopCapsuleSummary[] {
  if (tab === 'all') {
    return capsules;
  }
  return capsules.filter((capsule) => capsuleIsFavorited(capsule, state));
}

export function reduceCapsuleFavoriteToggle(
  capsules: DesktopCapsuleSummary[],
  state: CapsuleFavoritesState,
  capsuleId: string,
  favorited: boolean,
): CapsuleFavoriteTransition {
  const capsule = capsules.find((candidate) => candidate.id === capsuleId);
  if (!capsule) {
    return { capsules, state, effect: null };
  }
  const current = state.mutations[capsuleId] ?? initialMutation(capsule);
  if (current.desiredFavorited === favorited) {
    return { capsules, state, effect: null };
  }
  const mutation = { ...current, desiredFavorited: favorited };
  let favoritesGeneration = state.favoritesGeneration;
  let effect: CapsuleFavoriteEffect | null = null;
  if (!current.inFlight) {
    mutation.inFlight = true;
    favoritesGeneration += 1;
    effect = { capsuleId, favorited };
  }
  return {
    capsules,
    state: {
      favoritesGeneration,
      mutations: { ...state.mutations, [capsuleId]: mutation },
    },
    effect,
  };
}

export function reduceCapsuleFavoriteSuccess(
  capsules: DesktopCapsuleSummary[],
  state: CapsuleFavoritesState,
  capsuleId: string,
  result: SetCapsuleFavoriteResult,
): CapsuleFavoriteTransition {
  const current = state.mutations[capsuleId];
  if (!current?.inFlight) {
    return { capsules, state, effect: null };
  }

  const returnedServerFavorited = result.capsule.favoritedAt !== null;
  let favoritesGeneration = state.favoritesGeneration + 1;
  const mutation: CapsuleFavoriteMutationState = {
    serverFavoritedAt: result.capsule.favoritedAt,
    desiredFavorited: current.desiredFavorited,
    inFlight: false,
  };
  let effect: CapsuleFavoriteEffect | null = null;
  if (mutation.desiredFavorited !== returnedServerFavorited) {
    mutation.inFlight = true;
    favoritesGeneration += 1;
    effect = { capsuleId, favorited: mutation.desiredFavorited };
  } else {
    mutation.desiredFavorited = returnedServerFavorited;
  }

  return {
    capsules: replaceCapsule(capsules, result.capsule),
    state: {
      favoritesGeneration,
      mutations: { ...state.mutations, [capsuleId]: mutation },
    },
    effect,
  };
}

export function reduceCapsuleFavoriteFailure(
  capsules: DesktopCapsuleSummary[],
  state: CapsuleFavoritesState,
  capsuleId: string,
): CapsuleFavoriteTransition {
  const current = state.mutations[capsuleId];
  if (!current?.inFlight) {
    return { capsules, state, effect: null };
  }
  const mutation: CapsuleFavoriteMutationState = {
    ...current,
    desiredFavorited: serverFavorited(current),
    inFlight: false,
  };
  return {
    capsules: capsules.map((capsule) =>
      capsule.id === capsuleId
        ? { ...capsule, favoritedAt: mutation.serverFavoritedAt }
        : capsule,
    ),
    state: {
      favoritesGeneration: state.favoritesGeneration + 1,
      mutations: { ...state.mutations, [capsuleId]: mutation },
    },
    effect: null,
  };
}

export function mergeCapsuleFavoriteRefresh(
  currentCapsules: DesktopCapsuleSummary[],
  refreshedCapsules: DesktopCapsuleSummary[],
  state: CapsuleFavoritesState,
  capturedGeneration: number,
): CapsuleFavoriteTransition {
  const currentById = new Map(currentCapsules.map((capsule) => [capsule.id, capsule]));
  const mutations = { ...state.mutations };
  const generationIsCurrent = capturedGeneration === state.favoritesGeneration;
  const refreshedIds = new Set(refreshedCapsules.map((capsule) => capsule.id));

  const capsules = refreshedCapsules.map((refreshed) => {
    const currentCapsule = currentById.get(refreshed.id);
    const mutation = mutations[refreshed.id]
      ?? (currentCapsule ? initialMutation(currentCapsule) : null);
    const pending = mutation
      ? mutation.inFlight || mutation.desiredFavorited !== serverFavorited(mutation)
      : false;

    if (generationIsCurrent && !pending) {
      mutations[refreshed.id] = initialMutation(refreshed);
      return refreshed;
    }
    if (mutation) {
      mutations[refreshed.id] = mutation;
      return { ...refreshed, favoritedAt: mutation.serverFavoritedAt };
    }
    return refreshed;
  });

  for (const [capsuleId, mutation] of Object.entries(mutations)) {
    if (!refreshedIds.has(capsuleId) && !mutation.inFlight) {
      delete mutations[capsuleId];
    }
  }

  return {
    capsules,
    state: { ...state, mutations },
    effect: null,
  };
}
