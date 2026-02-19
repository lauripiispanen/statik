import { ClickableProps } from "./types";

export class Button {
  private label: string;
  private props: ClickableProps;

  constructor(label: string, props: ClickableProps = {}) {
    this.label = label;
    this.props = props;
  }

  render(): string {
    return `<button class="${this.props.className || ""}">${this.label}</button>`;
  }

  disable(): void {
    this.props.disabled = true;
  }
}
