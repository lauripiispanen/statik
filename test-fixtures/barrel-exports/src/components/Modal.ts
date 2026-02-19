import { ComponentProps } from "./types";

export class Modal {
  private title: string;
  private content: string;
  private props: ComponentProps;

  constructor(title: string, content: string, props: ComponentProps = {}) {
    this.title = title;
    this.content = content;
    this.props = props;
  }

  render(): string {
    return `<div class="modal"><h2>${this.title}</h2><p>${this.content}</p></div>`;
  }

  open(): void {
    console.log("Opening modal:", this.title);
  }

  close(): void {
    console.log("Closing modal:", this.title);
  }
}
