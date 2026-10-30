import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";

import { useAuth } from "./auth";
import {
  addSkillSource,
  getAvailableSkills,
  getSkillsState,
  installSkill,
  invokeSkill,
  removeSkillPack,
  removeSkillSource,
  syncSkills,
  type AvailableSkill,
  type SkillsState,
} from "./skillsApi";

export function useSkillsState(session?: string | null) {
  const { baseUrl } = useAuth();
  return useQuery<SkillsState>({
    queryKey: ["skills-state", baseUrl, session ?? null],
    queryFn: () => getSkillsState(baseUrl as string, session),
    enabled: baseUrl != null,
  });
}

export function useAvailableSkills(marketplace: string | null) {
  const { baseUrl } = useAuth();
  return useQuery<AvailableSkill[]>({
    queryKey: ["skills-available", baseUrl, marketplace],
    queryFn: () => getAvailableSkills(baseUrl as string, marketplace as string),
    enabled: baseUrl != null && marketplace != null,
    staleTime: 60_000,
  });
}

function useSkillsMutation<V, R>(run: (baseUrl: string, vars: V) => Promise<R>) {
  const { baseUrl } = useAuth();
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (vars: V) => run(baseUrl as string, vars),
    onSuccess: () => {
      void queryClient.invalidateQueries({ queryKey: ["skills", baseUrl] });
      void queryClient.invalidateQueries({ queryKey: ["skills-state", baseUrl] });
    },
  });
}

export const useInstallSkill = () => useSkillsMutation(installSkill);
export const useRemoveSkillPack = () => useSkillsMutation(removeSkillPack);
export const useAddSkillSource = () => useSkillsMutation(addSkillSource);
export const useRemoveSkillSource = () => useSkillsMutation(removeSkillSource);
export const useSyncSkills = () => useSkillsMutation(syncSkills);
export const useInvokeSkill = () => useSkillsMutation(invokeSkill);
