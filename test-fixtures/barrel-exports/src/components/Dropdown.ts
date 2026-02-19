import { ClickableProps } from "./types";
import { Select, SelectOption } from "./Select";

export class Dropdown {
  private select: Select;
  private props: ClickableProps;

  constructor(options: SelectOption[], props: ClickableProps = {}) {
    this.select = new Select(options);
    this.props = props;
  }

  render(): string {
    return `<div class="dropdown">${this.select.render()}</div>`;
  }
}
