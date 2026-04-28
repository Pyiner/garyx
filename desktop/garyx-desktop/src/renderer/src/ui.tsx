import type { ButtonHTMLAttributes, HTMLAttributes, ReactNode } from 'react';

function cx(...values: Array<string | false | null | undefined>) {
  return values.filter(Boolean).join(' ');
}

type ButtonProps = ButtonHTMLAttributes<HTMLButtonElement> & {
  variant?: 'default' | 'secondary' | 'outline' | 'ghost';
  size?: 'default' | 'sm' | 'lg';
};

export function UIButton({
  className,
  variant = 'default',
  size = 'default',
  type = 'button',
  ...props
}: ButtonProps) {
  return (
    <button
      className={cx('ui-button', `ui-button-${variant}`, `ui-button-${size}`, className)}
      type={type}
      {...props}
    />
  );
}

type CardProps = HTMLAttributes<HTMLDivElement> & {
  children: ReactNode;
};

export function UICard({ className, children, ...props }: CardProps) {
  return (
    <div className={cx('ui-card', className)} {...props}>
      {children}
    </div>
  );
}

export function UICardHeader({ className, children, ...props }: CardProps) {
  return (
    <div className={cx('ui-card-header', className)} {...props}>
      {children}
    </div>
  );
}

export function UICardTitle({ className, children, ...props }: CardProps) {
  return (
    <div className={cx('ui-card-title', className)} {...props}>
      {children}
    </div>
  );
}

export function UICardDescription({ className, children, ...props }: CardProps) {
  return (
    <div className={cx('ui-card-description', className)} {...props}>
      {children}
    </div>
  );
}

export function UICardContent({ className, children, ...props }: CardProps) {
  return (
    <div className={cx('ui-card-content', className)} {...props}>
      {children}
    </div>
  );
}

export function UIBadge({
  className,
  children,
  ...props
}: HTMLAttributes<HTMLSpanElement> & { children: ReactNode }) {
  return (
    <span className={cx('ui-badge', className)} {...props}>
      {children}
    </span>
  );
}
