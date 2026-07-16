export interface SaveImageInput {
  dataUrl: string;
  suggestedName?: string;
}

export type SaveImageResult =
  | {
      canceled: true;
    }
  | {
      canceled: false;
      filePath: string;
    };
