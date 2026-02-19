export function Injectable(): ClassDecorator {
  return (target: Function) => {
    Reflect.defineMetadata("injectable", true, target);
  };
}

export function Log(): MethodDecorator {
  return (target: Object, key: string | symbol, descriptor: PropertyDescriptor) => {
    const original = descriptor.value;
    descriptor.value = function (...args: unknown[]) {
      console.log(`Calling ${String(key)} with`, args);
      return original.apply(this, args);
    };
  };
}

export function Validate(): ParameterDecorator {
  return (target: Object, key: string | symbol | undefined, index: number) => {
    console.log(`Validating parameter ${index} of ${String(key)}`);
  };
}
