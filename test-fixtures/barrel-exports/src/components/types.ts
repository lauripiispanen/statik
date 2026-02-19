export interface ComponentProps {
  className?: string;
  id?: string;
  testId?: string;
}

export interface ClickableProps extends ComponentProps {
  onClick?: () => void;
  disabled?: boolean;
}

export interface DraggableProps extends ComponentProps {
  onDragStart?: () => void;
  onDragEnd?: () => void;
}
