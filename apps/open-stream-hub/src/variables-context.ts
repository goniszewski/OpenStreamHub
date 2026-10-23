import { createContext, useContext } from 'react';
import type { Variable } from './contracts';

// The variables the profile being edited declares, for the pickers of the actions and tests that name one.
export const VariablesContext = createContext<ReadonlyArray<Variable>>([]);
export const useVariables = (): ReadonlyArray<Variable> => useContext(VariablesContext);
