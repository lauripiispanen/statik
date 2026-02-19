// Input component - used via barrel import
import { ComponentProps } from "./types";

export class Input {
  private type: string;
  private placeholder: string;
  private props: ComponentProps;

  constructor(type: string, placeholder: string, props: ComponentProps = {}) {
    this.type = type;
    this.placeholder = placeholder;
    this.props = props;
  }

  render(): string {
    return `<input type="${this.type}" placeholder="${this.placeholder}" />`;
  }
}
