import { ComponentProps } from "./types";

export interface SelectOption {
  value: string;
  label: string;
}

export class Select {
  private options: SelectOption[];
  private props: ComponentProps;

  constructor(options: SelectOption[], props: ComponentProps = {}) {
    this.options = options;
    this.props = props;
  }

  render(): string {
    const optionsHtml = this.options
      .map((o) => `<option value="${o.value}">${o.label}</option>`)
      .join("");
    return `<select>${optionsHtml}</select>`;
  }
}
