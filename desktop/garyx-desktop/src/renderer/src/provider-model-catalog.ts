export type ProviderModelCatalogRequestState<ProviderType extends string> = {
  catalogs: Partial<Record<ProviderType, unknown>>;
  requests: Partial<Record<ProviderType, unknown>>;
  attempted: Partial<Record<ProviderType, boolean>>;
};

export type ProviderModelCatalogRequestOptions = {
  retry?: boolean;
};

export function shouldRequestProviderModelCatalog<ProviderType extends string>(
  state: ProviderModelCatalogRequestState<ProviderType>,
  providerType: ProviderType,
  options: ProviderModelCatalogRequestOptions = {},
): boolean {
  if (state.catalogs[providerType]) {
    return false;
  }
  if (state.requests[providerType]) {
    return false;
  }
  if (!options.retry && state.attempted[providerType]) {
    return false;
  }
  return true;
}
